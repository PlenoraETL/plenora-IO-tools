//! La validazione delle geometrie, riga per riga e batch per batch.
//!
//! Estratto da `driver.rs` per M1. E' il gruppo con un solo riferimento in
//! entrata da quello della scrittura -- `validate_geometry_batch_at` -- e
//! nessuno in uscita: la misura che ha motivato questa separazione dava zero
//! per tutte le altre direzioni.
//!
//! Lo spostamento e' meccanico: nessuna firma cambia, nessun ordine di
//! operazioni cambia, e cio' che serve dal genitore arriva per `use super::*`.
//!
//! Qui non ci sono risorse con rilascio osservabile: le funzioni ispezionano
//! batch e restituiscono violazioni. I lease e lo staging stanno nel gruppo
//! della scrittura, e non si sono mossi.

use arrow_array::Array;
use plenora_io_model::Result;

use super::{
    inspect_wkb, saturating_u64, BTreeMap, BinaryArray, CapabilityReason, ContractIdentifier,
    CoordinateDimensions, ErrorCategory, ErrorPhase, GeometryColumnContract, GeometryEncoding,
    GeometryWriteSupport, IoErrorCode, KnownOrUnknownCount, LargeBinaryArray, PlenoraIoError,
    PublicMessage, RecordBatch, RemoteEffect, RetryDisposition, RowDiagnosticColumn,
    RowDiagnosticExample, RowDiagnosticScope, RowDiagnosticWriteOutcome, RowDiagnosticWriteState,
    RowDiagnostics, RowDiagnosticsCompleteness, WkbInspection, WkbLimits,
    WriteDiagnosticStateCounts, ROW_DIAGNOSTICS_CONTRACT, ROW_DIAGNOSTICS_INDEX_BASIS,
    ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE,
};

fn geometry_violation(
    driver: &'static str,
    field: Option<&ContractIdentifier>,
    reason: CapabilityReason,
    detail: &PublicMessage,
) -> PlenoraIoError {
    PlenoraIoError::capability_redatta(driver, field, reason, detail)
}

fn validate_inspected_geometry(
    driver: &'static str,
    support: GeometryWriteSupport,
    contract: &GeometryColumnContract,
    geometry: &WkbInspection,
) -> Result<()> {
    let actual_dimensions = geometry.dimensions;
    if contract.dimensions != CoordinateDimensions::Unknown
        && contract.dimensions != actual_dimensions
    {
        return Err(geometry_violation(
            driver,
            ContractIdentifier::from_geometry_column(contract).as_ref(),
            CapabilityReason::CoordinateDimensions,
            &PublicMessage::CuratedPair(
                "dimensioni del payload diverse da quelle dichiarate dal contratto:",
                actual_dimensions.nome(),
            ),
        ));
    }
    if !support.dimensions.contains(&actual_dimensions) {
        return Err(geometry_violation(
            driver,
            ContractIdentifier::from_geometry_column(contract).as_ref(),
            CapabilityReason::CoordinateDimensions,
            &PublicMessage::CuratedPair(
                "dimensioni del payload non supportate dal driver:",
                actual_dimensions.nome(),
            ),
        ));
    }

    let allow_srid = contract.encoding == GeometryEncoding::Ewkb;
    if !geometry.nested_dimensions_coherent || (!allow_srid && geometry.contains_srid) {
        return Err(geometry_violation(
            driver,
            ContractIdentifier::from_geometry_column(contract).as_ref(),
            if allow_srid {
                CapabilityReason::CoordinateDimensions
            } else {
                CapabilityReason::GeometryEncoding
            },
            &PublicMessage::Curated(
                "componenti WKB con dimensioni incoerenti o SRID EWKB non dichiarato",
            ),
        ));
    }
    if contract.encoding == GeometryEncoding::Ewkb && geometry.srid != contract.srid {
        return Err(geometry_violation(
            driver,
            ContractIdentifier::from_geometry_column(contract).as_ref(),
            CapabilityReason::GeometryEncoding,
            // Gli SRID non entrano: sono numeri **letti dal payload**, e il
            // vincolo di S9 ammette solo indici, conteggi, tetti e codici
            // strutturali.
            &PublicMessage::Curated("SRID del payload diverso da quello dichiarato"),
        ));
    }
    if !contract.geometry_types.is_empty()
        && !contract.geometry_types.contains(&geometry.geometry_type)
    {
        return Err(geometry_violation(
            driver,
            ContractIdentifier::from_geometry_column(contract).as_ref(),
            CapabilityReason::MixedGeometry,
            &PublicMessage::CuratedPair(
                "tipo geometrico assente da quelli dichiarati:",
                geometry.geometry_type.canonical_name(),
            ),
        ));
    }
    Ok(())
}

pub fn validate_geometry_batch_at(
    driver: &'static str,
    support: GeometryWriteSupport,
    contract: Option<&GeometryColumnContract>,
    batch: &RecordBatch,
    wkb_limits: WkbLimits,
    row_offset: u64,
    input_total: Option<u64>,
) -> Result<u64> {
    let Some(contract) = contract else {
        let violations = nullability_violations(batch, row_offset)?;
        return if violations.is_empty() {
            Ok(0)
        } else {
            Err(write_rejection_error(
                driver,
                saturating_u64(batch.num_rows()),
                row_offset,
                &violations,
                input_total,
            ))
        };
    };
    let index = batch
        .schema()
        .fields()
        .iter()
        .position(|field| field.name() == &contract.name)
        .ok_or_else(|| {
            geometry_violation(
                driver,
                ContractIdentifier::from_geometry_column(contract).as_ref(),
                CapabilityReason::GeometryNotSupported,
                &PublicMessage::Curated("colonna geometrica dichiarata assente dal batch"),
            )
        })?;
    let array = batch.column(index);

    let mut violations = nullability_violations(batch, row_offset)?;
    let mut components = 0_u64;
    if let Some(values) = array.as_any().downcast_ref::<BinaryArray>() {
        for row in 0..values.len() {
            inspect_geometry_row(
                driver,
                support,
                contract,
                &wkb_limits,
                row,
                if values.is_null(row) {
                    None
                } else {
                    Some(values.value(row))
                },
                row_offset,
                &mut violations,
                &mut components,
            )?;
        }
    } else if let Some(values) = array.as_any().downcast_ref::<LargeBinaryArray>() {
        for row in 0..values.len() {
            inspect_geometry_row(
                driver,
                support,
                contract,
                &wkb_limits,
                row,
                if values.is_null(row) {
                    None
                } else {
                    Some(values.value(row))
                },
                row_offset,
                &mut violations,
                &mut components,
            )?;
        }
    } else {
        return Err(geometry_violation(
            driver,
            ContractIdentifier::from_geometry_column(contract).as_ref(),
            CapabilityReason::GeometryEncoding,
            &PublicMessage::Curated("colonna geometrica runtime non Binary/LargeBinary"),
        ));
    }
    if violations.is_empty() {
        Ok(components)
    } else {
        Err(write_rejection_error(
            driver,
            saturating_u64(batch.num_rows()),
            row_offset,
            &violations,
            input_total,
        ))
    }
}

#[derive(Clone)]
pub struct WriteRowViolation {
    pub source_index: u64,
    pub cause: &'static str,
    pub column: String,
    pub capability_reason: CapabilityReason,
}

fn nullability_violations(
    batch: &RecordBatch,
    row_offset: u64,
) -> Result<BTreeMap<u64, WriteRowViolation>> {
    let mut violations = BTreeMap::new();
    for (column_index, field) in batch.schema().fields().iter().enumerate() {
        if field.is_nullable() {
            continue;
        }
        let array = batch.column(column_index);
        for row in 0..batch.num_rows() {
            if array.is_null(row) {
                let source_index = row_offset
                    .checked_add(u64::try_from(row).map_err(|_| {
                        PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                            "indice riga oltre u64",
                        ))
                    })?)
                    .ok_or_else(|| {
                        PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                            "overflow nell'indice riga",
                        ))
                    })?;
                violations
                    .entry(source_index)
                    .or_insert_with(|| WriteRowViolation {
                        source_index,
                        cause: "contract.nullability",
                        column: field.name().to_owned(),
                        capability_reason: CapabilityReason::Nullability,
                    });
            }
        }
    }
    Ok(violations)
}

#[allow(clippy::too_many_arguments)]
fn inspect_geometry_row(
    driver: &'static str,
    support: GeometryWriteSupport,
    contract: &GeometryColumnContract,
    limits: &plenora_io_model::limits::WkbLimits,
    row: usize,
    bytes: Option<&[u8]>,
    row_offset: u64,
    violations: &mut BTreeMap<u64, WriteRowViolation>,
    components: &mut u64,
) -> Result<()> {
    let source_index = row_offset
        .checked_add(u64::try_from(row).map_err(|_| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated("indice riga oltre u64"))
        })?)
        .ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated("overflow nell'indice riga"))
        })?;
    if violations.contains_key(&source_index) {
        return Ok(());
    }
    let Some(bytes) = bytes else {
        if !contract.nullable {
            violations.insert(
                source_index,
                WriteRowViolation {
                    source_index,
                    cause: "contract.nullability",
                    column: contract.name.clone(),
                    capability_reason: CapabilityReason::Nullability,
                },
            );
        }
        return Ok(());
    };
    let Ok(inspection) = inspect_wkb(bytes, limits) else {
        violations.insert(
            source_index,
            WriteRowViolation {
                source_index,
                cause: "conversion.invalid_geometry",
                column: contract.name.clone(),
                capability_reason: CapabilityReason::GeometryEncoding,
            },
        );
        return Ok(());
    };
    if let Err(error) = validate_inspected_geometry(driver, support, contract, &inspection) {
        let capability_reason = error
            .capability_reason
            .unwrap_or(CapabilityReason::GeometryEncoding);
        let cause = match capability_reason {
            CapabilityReason::Nullability => "contract.nullability",
            CapabilityReason::CoordinateDimensions => "contract.coordinate_dimensions",
            CapabilityReason::MixedGeometry => "contract.geometry_type",
            _ => "contract.geometry_encoding",
        };
        violations.insert(
            source_index,
            WriteRowViolation {
                source_index,
                cause,
                column: contract.name.clone(),
                capability_reason,
            },
        );
        return Ok(());
    }
    *components = components
        .checked_add(u64::try_from(inspection.components).map_err(|_| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "geometria oltre il conteggio supportato",
            ))
        })?)
        .ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "overflow nel conteggio dei componenti geometrici",
            ))
        })?;
    Ok(())
}

/// L'errore di righe rifiutate quando il report non e' emettibile.
///
/// Conserva **categoria, fase e causa** del rifiuto reale: quelle non dipendono
/// da `input_total`, che governa solo il report. La causa passa nel messaggio
/// perche' senza report non avrebbe dove stare, ed e' un vocabolario chiuso
/// (`contract.nullability`, `conversion.invalid_geometry`, ...) — non un valore
/// derivato dal payload, che non potrebbe uscire.
///
/// Il totale **resta assente**. Il report non viene allegato invece di essere
/// allegato con un totale inventato: un report che dichiara un totale che
/// nessuno ha dichiarato e' peggio di un report che non c'e'.
fn errore_di_rifiuto_senza_report(
    driver: &'static str,
    causa: Option<&'static str>,
    capability_reason: Option<CapabilityReason>,
) -> PlenoraIoError {
    // Il driver esce dal campo `driver`, non dal testo: ripeterlo nel
    // messaggio non aggiungeva niente e faceva sembrare interpolato un valore
    // che era gia' strutturato. La causa resta, ed e' un vocabolario chiuso di
    // `&'static str`.
    let messaggio = causa.map_or(
        PublicMessage::Curated("righe rifiutate prima della scrittura"),
        |causa| PublicMessage::CuratedPair("righe rifiutate prima della scrittura:", causa),
    );
    let mut error = PlenoraIoError::redatto(
        IoErrorCode::Generic,
        ErrorCategory::DataMapping,
        ErrorPhase::Write,
        RemoteEffect::None,
        RetryDisposition::Never,
        &messaggio,
    );
    error.driver = Some(driver.to_owned());
    error.capability_reason = capability_reason;
    error
}

pub fn write_rejection_error(
    driver: &'static str,
    _batch_rows: u64,
    row_offset: u64,
    violations: &BTreeMap<u64, WriteRowViolation>,
    input_total: Option<u64>,
) -> PlenoraIoError {
    const EXAMPLES_LIMIT: u64 = 64;
    let prima = violations.values().next();
    let first_reason = prima.map(|violation| violation.capability_reason);
    let Some(input_total) = input_total.filter(|total| *total > 0) else {
        // Senza `input_total` il **report** non e' emettibile — il contratto
        // `plenora-io-row-diagnostics-v1` lo pretende positivo, e inventarlo
        // sarebbe peggio che ometterlo. L'**errore** pero' esiste comunque, ed
        // e' lo stesso: righe rifiutate prima della scrittura.
        //
        // Prima si restituiva un `Contract` sull'`input_total` mancante, cioe'
        // si sostituiva la causa primaria con una condizione dell'infrastruttura
        // diagnostica. Chi leggeva l'errore vedeva un problema interno al posto
        // del proprio: la riga era invalida, e il messaggio parlava d'altro.
        return errore_di_rifiuto_senza_report(
            driver,
            prima.map(|violation| violation.cause),
            first_reason,
        );
    };
    let observed_total = saturating_u64(violations.len());
    let mut counts = BTreeMap::new();
    for violation in violations.values() {
        *counts.entry(violation.cause.to_owned()).or_insert(0_u64) += 1;
    }
    let mut column_name_unattestable = false;
    // `EXAMPLES_LIMIT` e' la costante letterale 64: la conversione e' esatta
    // su ogni target supportato.
    #[allow(clippy::cast_possible_truncation)]
    let examples = violations
        .values()
        .take(EXAMPLES_LIMIT as usize)
        .map(|violation| {
            let column = RowDiagnosticColumn::attest(violation.column.clone());
            column_name_unattestable |= !column.is_attested();
            RowDiagnosticExample {
                source_index: violation.source_index,
                cause: violation.cause.to_owned(),
                column: column.into_option(),
                key: None,
                write_state: Some(RowDiagnosticWriteState::CertainlyRejected),
            }
        })
        .collect::<Vec<_>>();
    let diagnostics = RowDiagnostics {
        contract: ROW_DIAGNOSTICS_CONTRACT.to_owned(),
        scope: RowDiagnosticScope::Write,
        index_basis: ROW_DIAGNOSTICS_INDEX_BASIS.to_owned(),
        completeness: RowDiagnosticsCompleteness::Partial,
        knowledge_limits: Some({
            let mut limits = vec!["write_validation_stopped_at_first_rejected_batch".to_owned()];
            if column_name_unattestable {
                limits.push(ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE.to_owned());
            }
            limits
        }),
        observed_total,
        total: None,
        input_total: Some(input_total),
        counts,
        examples_limit: EXAMPLES_LIMIT,
        examples_truncated: observed_total > EXAMPLES_LIMIT,
        examples,
        diagnostic_state_counts: Some(WriteDiagnosticStateCounts {
            certainly_rejected: observed_total,
            certainly_not_attempted: 0,
            certainly_rolled_back: 0,
            effect_unknown: 0,
        }),
        write_outcome: Some(RowDiagnosticWriteOutcome {
            certainly_rejected: KnownOrUnknownCount::Known {
                value: observed_total,
            },
            certainly_not_attempted: KnownOrUnknownCount::Known {
                value: input_total
                    .saturating_sub(row_offset)
                    .saturating_sub(observed_total),
            },
            certainly_rolled_back: KnownOrUnknownCount::Unknown,
            effect_unknown: KnownOrUnknownCount::Unknown,
        }),
    };
    let mut error = PlenoraIoError::redatto(
        IoErrorCode::Generic,
        ErrorCategory::DataMapping,
        ErrorPhase::Write,
        RemoteEffect::None,
        RetryDisposition::Never,
        &PublicMessage::Curated("righe rifiutate prima della scrittura"),
    );
    error = error.with_row_diagnostics(diagnostics);
    error.driver = Some(driver.to_owned());
    error.capability_reason = first_reason;
    error
}
