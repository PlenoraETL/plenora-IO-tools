//! Il macchinario della scrittura: writer limitato, risorse del batch, staging.
//!
//! Estratto da `driver.rs` per M1. E' l'unico dei tre gruppi che ne chiama un
//! altro -- `validate_geometry_batch_at`, una volta sola -- e quella chiamata
//! e' ora un `use` visibile invece di una fra vicini.
//!
//! # Perche' questo spostamento e' il piu' delicato dei tre
//!
//! Qui vivono le risorse con rilascio osservabile: `LimitedWriter` e
//! `WriteBatchResources` tengono lease di memoria, spill e concorrenza, e il
//! momento in cui un `Drop` li restituisce e' parte del comportamento, non un
//! dettaglio. Un'estrazione che cambiasse l'ordine di distruzione, o che
//! spostasse un ritorno anticipato oltre un rilascio, non si vedrebbe in nessun
//! conteggio di riferimenti.
//!
//! Lo spostamento e' percio' **solo** un cambio di file: nessuna riga e' stata
//! riordinata, nessun `drop` esplicito aggiunto o tolto, nessun ritorno
//! anticipato spostato. Cio' che il gruppo usa del genitore arriva per
//! `use super::*`.

use plenora_io_model::Result;

use super::{
    check_cancelled, incremental_batch_memory_size, saturating_u64, saturating_usize,
    validate_geometry_batch_at, CancellationToken, CapabilityReason, ConcurrencyLease,
    CountedLease, ErrorCategory, ErrorPhase, FidelityAssessment, FormatWriter,
    GeometryColumnContract, GeometryWriteSupport, InternalMemoryLease, IoErrorCode, LayerId,
    LossReport, NumeroStrutturale, OperationBudget, OperationCounter, PlenoraIoError,
    PublicMessage, Published, RecordBatch, RemoteEffect, RetryDisposition, SchemaRef,
    WriteLimitsView,
};

pub struct GeometryValidation {
    pub driver: &'static str,
    pub support: GeometryWriteSupport,
    pub layers: Vec<Option<GeometryColumnContract>>,
}

pub struct LimitedWriter {
    pub inner: Box<dyn FormatWriter>,
    pub driver: &'static str,
    pub limits: WriteLimitsView,
    pub rows: usize,
    pub layer_rows: Vec<u64>,
    pub input_totals: Vec<Option<u64>>,
    pub failed: bool,
    pub contracts: Vec<SchemaRef>,
    pub geometry_validation: Option<GeometryValidation>,
    pub fidelity: FidelityAssessment,
    pub planned_loss: LossReport,
    pub cancellation: CancellationToken,
    pub budget: OperationBudget,
    pub _operation_lease: Option<ConcurrencyLease>,
}

struct WriteBatchResources {
    rows: u64,
    bytes: u64,
    rows_lease: Option<CountedLease>,
    output_lease: Option<CountedLease>,
    /// La memoria dello staging del writer: prenotazione viva, restituita al
    /// drop come ogni occupazione interna (INV-5).
    memory_lease: Option<InternalMemoryLease>,
    geometry_components: u64,
    geometry_lease: Option<CountedLease>,
}

impl WriteBatchResources {
    fn commit(self) -> Result<()> {
        if let Some(rows_lease) = self.rows_lease {
            rows_lease.commit(self.rows)?;
        }
        if let Some(output_lease) = self.output_lease {
            output_lease.commit(self.bytes)?;
        }
        drop(self.memory_lease);
        if self.geometry_components > 0 {
            self.geometry_lease
                .ok_or_else(|| {
                    PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                        "budget geometrico esaurito",
                    ))
                })?
                .commit(self.geometry_components)?;
        }
        Ok(())
    }
}

impl LimitedWriter {
    // Sequenza lineare di contabilizzazioni e guardie, una per limite: la
    // lunghezza e' nel numero di limiti, non in complessita' logica.
    #[allow(clippy::too_many_lines)]
    fn account(&mut self, layer: usize, batch: &RecordBatch) -> Result<WriteBatchResources> {
        self.budget.context().ensure_active()?;
        if let Some(contract) = self.contracts.get(layer) {
            if batch.schema().as_ref() != contract.as_ref() {
                // `redatto` con `Generic` e non `schema_redatto`: il sito usava
                // `PlenoraIoError::new`, che imposta `code = Generic`, mentre
                // `schema_redatto` imposta `code = Schema`. S9 non cambia il wire, e
                // `code` e' parte della chiave di compatibilita' ratificata insieme a
                // category, phase e retry.
                //
                // `Schema` sarebbe piu' preciso di `Generic` per una discordanza di
                // schema, ma renderlo tale e' una decisione da ratificare, non una
                // conseguenza di un refactor sui messaggi.
                return Err(PlenoraIoError::redatto(
                    IoErrorCode::Generic,
                    ErrorCategory::Schema,
                    ErrorPhase::Validate,
                    RemoteEffect::None,
                    RetryDisposition::Never,
                    &PublicMessage::CuratedWith(
                        "batch diverso dal contratto dichiarato (schema, ordine, tipi, \
                         nullability o metadata) al layer",
                        NumeroStrutturale::Indice(saturating_u64(layer)),
                    ),
                ));
            }
        } else if !self.contracts.is_empty() {
            return Err(PlenoraIoError::capability_redatta(
                self.driver,
                None,
                CapabilityReason::MultipleLayers,
                &layer_fuori_dal_piano(saturating_u64(layer)),
            ));
        }
        if batch.num_columns() > self.limits.max_columns {
            return Err(PlenoraIoError::limite_redatto(
                &PublicMessage::CuratedBetween(
                    "batch con",
                    NumeroStrutturale::Conteggio(saturating_u64(batch.num_columns())),
                    "colonne oltre il limite di",
                    NumeroStrutturale::Limite(saturating_u64(self.limits.max_columns)),
                ),
            ));
        }
        let batch_rows = u64::try_from(batch.num_rows()).map_err(|_| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "batch oltre il conteggio supportato",
            ))
        })?;
        let layer_rows = *self.layer_rows.get(layer).ok_or_else(|| {
            PlenoraIoError::contratto_redatto(&layer_fuori_dal_piano(saturating_u64(layer)))
        })?;
        if self
            .input_totals
            .get(layer)
            .copied()
            .flatten()
            .is_some_and(|total| {
                layer_rows
                    .checked_add(batch_rows)
                    .is_none_or(|rows| rows > total)
            })
        {
            return Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                "write oltre input_total dichiarato",
            )));
        }
        self.rows = self.rows.checked_add(batch.num_rows()).ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "overflow nel conteggio delle righe",
            ))
        })?;
        if self.rows > self.limits.max_rows {
            return Err(PlenoraIoError::limite_redatto(
                &PublicMessage::CuratedBetween(
                    "scritte",
                    NumeroStrutturale::Conteggio(saturating_u64(self.rows)),
                    "righe oltre il limite di",
                    NumeroStrutturale::Limite(saturating_u64(self.limits.max_rows)),
                ),
            ));
        }
        let geometry_components = if let Some(validation) = &self.geometry_validation {
            let mut effective_limits = self.limits;
            effective_limits.wkb.max_cell_bytes = effective_limits
                .wkb
                .max_cell_bytes
                .min(self.budget.context().limits().max_wkb_cell_bytes());
            effective_limits.wkb.max_components = effective_limits.wkb.max_components.min(
                saturating_usize(self.budget.remaining(OperationCounter::GeometryComponents)),
            );
            effective_limits.wkb.max_depth = effective_limits
                .wkb
                .max_depth
                .min(self.budget.context().limits().max_wkb_depth());
            validate_geometry_batch_at(
                validation.driver,
                validation.support,
                validation
                    .layers
                    .get(layer)
                    .ok_or_else(|| {
                        PlenoraIoError::capability_redatta(
                            validation.driver,
                            None,
                            CapabilityReason::MultipleLayers,
                            &layer_fuori_dal_piano(saturating_u64(layer)),
                        )
                    })?
                    .as_ref(),
                batch,
                effective_limits.wkb,
                *self.layer_rows.get(layer).ok_or_else(|| {
                    PlenoraIoError::contratto_redatto(&layer_fuori_dal_piano(saturating_u64(layer)))
                })?,
                self.input_totals.get(layer).copied().flatten(),
            )?
        } else {
            0
        };
        let rows = batch_rows;
        if rows == 0 {
            return Ok(WriteBatchResources {
                rows: 0,
                bytes: 0,
                rows_lease: None,
                output_lease: None,
                memory_lease: None,
                geometry_components: 0,
                geometry_lease: None,
            });
        }
        let bytes = u64::try_from(incremental_batch_memory_size(batch)).map_err(|_| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "batch oltre il conteggio byte supportato",
            ))
        })?;
        Ok(WriteBatchResources {
            rows,
            bytes,
            rows_lease: Some(self.budget.try_lease(OperationCounter::Rows, rows)?),
            output_lease: (bytes > 0)
                .then(|| self.budget.try_lease(OperationCounter::OutputBytes, bytes))
                .transpose()?,
            memory_lease: (bytes > 0)
                .then(|| self.budget.context().lease_memory_internal(bytes))
                .transpose()?,
            geometry_components,
            geometry_lease: (geometry_components > 0)
                .then(|| {
                    self.budget
                        .try_lease(OperationCounter::GeometryComponents, geometry_components)
                })
                .transpose()?,
        })
    }
}

impl FormatWriter for LimitedWriter {
    fn fidelity_assessment(&self) -> FidelityAssessment {
        self.fidelity.clone()
    }

    fn declare_input_total(&mut self, layer: LayerId, total: u64) -> Result<()> {
        let layer_index = layer.0 as usize;
        if self
            .layer_rows
            .get(layer_index)
            .is_some_and(|rows| *rows > 0)
        {
            return Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                "input_total deve essere dichiarato prima del primo write del layer",
            )));
        }
        let slot = self.input_totals.get(layer_index).ok_or_else(|| {
            PlenoraIoError::contratto_redatto(&layer_fuori_dal_piano(u64::from(layer.0)))
        })?;
        if slot.is_some_and(|declared| declared != total) {
            return Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                "input_total dichiarato in modo incoerente",
            )));
        }
        self.inner.declare_input_total(layer, total)?;
        self.input_totals[layer_index] = Some(total);
        Ok(())
    }

    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        check_cancelled(&self.cancellation, ErrorPhase::Write)?;
        if self.failed {
            return Err(PlenoraIoError::formato_redatto(
                self.driver,
                &PublicMessage::Curated("writer invalidato da un precedente errore di scrittura"),
            )
            .during(plenora_io_model::ErrorPhase::Write));
        }
        let result = self.account(0, batch).and_then(|resources| {
            let rows = resources.rows;
            self.inner.write(batch)?;
            resources.commit()?;
            self.layer_rows[0] = self.layer_rows[0].checked_add(rows).ok_or_else(|| {
                PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "overflow nel conteggio righe layer",
                ))
            })?;
            Ok(())
        });
        // Il driver scrive, e il tempo passa: una quota esaurita dentro la
        // scrittura dopo la scadenza e' una scadenza.
        let result = result.map_err(|errore| {
            super::limite_o_scadenza(self.budget.context(), errore, ErrorPhase::Write)
        });
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn write_to_layer(&mut self, layer: LayerId, batch: &RecordBatch) -> Result<()> {
        check_cancelled(&self.cancellation, ErrorPhase::Write)?;
        if self.failed {
            return Err(PlenoraIoError::formato_redatto(
                self.driver,
                &PublicMessage::Curated("writer invalidato da un precedente errore di scrittura"),
            )
            .during(plenora_io_model::ErrorPhase::Write));
        }
        let result = self.account(layer.0 as usize, batch).and_then(|resources| {
            let rows = resources.rows;
            self.inner.write_to_layer(layer, batch)?;
            resources.commit()?;
            let layer_rows = self.layer_rows.get_mut(layer.0 as usize).ok_or_else(|| {
                PlenoraIoError::contratto_redatto(&layer_fuori_dal_piano(u64::from(layer.0)))
            })?;
            *layer_rows = layer_rows.checked_add(rows).ok_or_else(|| {
                PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "overflow nel conteggio righe layer",
                ))
            })?;
            Ok(())
        });
        // Il driver scrive, e il tempo passa: una quota esaurita dentro la
        // scrittura dopo la scadenza e' una scadenza.
        let result = result.map_err(|errore| {
            super::limite_o_scadenza(self.budget.context(), errore, ErrorPhase::Write)
        });
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn finish(self: Box<Self>) -> Result<Published> {
        check_cancelled(&self.cancellation, ErrorPhase::Finalize)?;
        self.budget.context().ensure_active()?;
        if self.failed {
            return Err(PlenoraIoError::formato_redatto(
                self.driver,
                &PublicMessage::Curated("finish vietato dopo un errore di scrittura"),
            )
            .during(plenora_io_model::ErrorPhase::Finalize));
        }
        if self
            .input_totals
            .iter()
            .zip(&self.layer_rows)
            .any(|(declared, observed)| declared.is_some_and(|total| total != *observed))
        {
            return Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                "EOF prima dell'input_total esatto dichiarato",
            )));
        }
        // La finalizzazione del backend misura l'output: una quota superata
        // dopo che la scadenza e' passata dentro `finish` e' una scadenza.
        let contesto = self.budget.context().clone();
        let mut published = self
            .inner
            .finish()
            .map_err(|errore| super::limite_o_scadenza(&contesto, errore, ErrorPhase::Finalize))?;
        published.loss.merge(&self.planned_loss);
        published.fidelity = self.fidelity.with_loss_report(&published.loss);
        Ok(published)
    }
}

/// Il messaggio del layer runtime che il `WritePlan` non dichiara.
///
/// Cinque siti lo producevano con altrettanti `format!` identici. Uno solo, e
/// il numero e' un indice strutturale: non viene dal payload, viene dal piano.
const fn layer_fuori_dal_piano(layer: u64) -> PublicMessage {
    PublicMessage::CuratedWith(
        "fuori dal WritePlan il layer runtime",
        NumeroStrutturale::Indice(layer),
    )
}
