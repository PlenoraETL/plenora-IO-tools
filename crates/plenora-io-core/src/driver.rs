//! Il confine plug-in: `FormatDriver` + handle/reader/writer (ADR-IO 1).

use std::collections::BTreeMap;
use std::path::PathBuf;

use arrow_array::{Array, BinaryArray, LargeBinaryArray, RecordBatch};
use arrow_schema::DataType;
use plenora_io_model::contract::{
    CoordinateDimensions, FieldId, GeometryColumnContract, GeometryEncoding, GeometryType,
    LayerContract, LayerId,
};
use plenora_io_model::crs::CrsResolution;
use plenora_io_model::geometry::{is_geometry_field, read_geometry_contract_metadata};
use plenora_io_model::limits::Limits;
use plenora_io_model::wkb::{decode_wkb, WkbGeometry, WkbValue};
use plenora_io_model::{
    CancellationReason, CancellationToken, CapabilityReason, ErrorPhase, PlenoraIoError, Result,
};

use crate::descriptor::{
    ArrowTypeClass, AttributeWriteSupport, CrsWriteSupport, FormatDescriptor, GeometryWriteSupport,
    NullabilitySupport, TypeCoercionPolicy,
};
use crate::loss::{FidelityAssessment, FidelityReasonCode, LossExample, LossReport};
#[cfg(test)]
use crate::request::BatchTarget;
use crate::request::{ReadRequest, WritePlan};

mod batch_worker;
mod reader_adapters;
pub use batch_worker::{spawn_batch_reader, BatchEmitter};
pub use reader_adapters::{with_batch_target, with_cancellation, SingleReaderGate};

/// Sorgente di lettura (scheletro Fase 0).
pub enum Source {
    Path(PathBuf),
}

impl Source {
    /// Risolve la sorgente e applica il limite complessivo prima che un parser
    /// possa materializzarla. Le directory-dataset sono conteggiate senza
    /// seguire symlink.
    pub fn into_path_checked(
        self,
        limits: &Limits,
        cancellation: &CancellationToken,
    ) -> Result<PathBuf> {
        let Self::Path(path) = self;
        let mut total = 0_u64;
        let mut pending = vec![path.clone()];
        while let Some(candidate) = pending.pop() {
            check_cancelled(cancellation, ErrorPhase::Probe)?;
            let metadata = std::fs::symlink_metadata(&candidate)?;
            if metadata.file_type().is_symlink() {
                return Err(PlenoraIoError::Unsupported(
                    "symlink non ammesso nella sorgente".to_owned(),
                ));
            }
            if metadata.is_dir() {
                for entry in std::fs::read_dir(&candidate)? {
                    pending.push(entry?.path());
                }
            } else if metadata.is_file() {
                total = total.checked_add(metadata.len()).ok_or_else(|| {
                    PlenoraIoError::LimitExceeded("overflow nel conteggio dell'input".to_owned())
                })?;
                if total > limits.max_input_bytes {
                    return Err(PlenoraIoError::LimitExceeded(format!(
                        "input da {total} byte oltre il limite di {}",
                        limits.max_input_bytes
                    )));
                }
            }
        }
        Ok(path)
    }
}

/// Destinazione di scrittura (scheletro Fase 0).
pub enum Sink {
    /// File singolo o directory-dataset (multi-file), risolto dal driver.
    Path(PathBuf),
}

#[derive(Default)]
pub struct ReadOptions {
    /// CRS dichiarato per i formati che non lo portano (CSV/XLSX) — ADR-IO 4.
    pub assume_crs: Option<String>,
    /// Knob specifici del driver (es. csv: x_column/y_column/wkt_column/delimiter).
    pub format_options: BTreeMap<String, String>,
    /// Limiti condivisi del bordo I/O.
    pub limits: Limits,
    pub cancellation: CancellationToken,
}

#[derive(Default)]
pub struct WriteOptions {
    /// Profilo `DurableAtomicPublish` (fsync) invece di `AtomicPublish` — ADR-IO 2.
    pub durable: bool,
    /// Knob specifici del driver.
    pub format_options: BTreeMap<String, String>,
    /// Limiti condivisi del bordo I/O.
    pub limits: Limits,
    pub cancellation: CancellationToken,
}

pub fn check_cancelled(token: &CancellationToken, phase: ErrorPhase) -> Result<()> {
    match token.reason() {
        None => Ok(()),
        Some(CancellationReason::Deadline) => Err(PlenoraIoError::cancelled(phase, true)),
        Some(CancellationReason::Requested | CancellationReason::Parent) => {
            Err(PlenoraIoError::cancelled(phase, false))
        }
    }
}

/// Frequenza comune dei controlli cooperativi nei loop che materializzano.
/// È una potenza di due per mantenere trascurabile il costo del fast path.
pub const CANCELLATION_CHECK_INTERVAL: usize = 1024;

/// Controlla periodicamente il token senza imporre una lettura atomica per
/// ogni riga. Il chiamante deve passare un indice monotono a partire da zero.
pub fn check_cancelled_periodically(
    token: &CancellationToken,
    phase: ErrorPhase,
    index: usize,
) -> Result<()> {
    if index & (CANCELLATION_CHECK_INTERVAL - 1) == 0 {
        check_cancelled(token, phase)?;
    }
    Ok(())
}

pub trait FormatDriver: Send + Sync {
    fn descriptor(&self) -> &FormatDescriptor;
    /// Statico: header/schema/CRS, nessuna riga.
    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>>;
    /// Statico: verifica che il contratto sia rappresentabile (ADR-IO 3).
    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>>;
}

pub trait OpenDatasetHandle: Send + Sync {
    fn layers(&self) -> &[LayerContract];
    /// Valutazione di fedeltà concreta per il dataset aperto (ADR-IO 5).
    fn fidelity_assessment(&self) -> FidelityAssessment;
    /// Apre un reader indipendente per un layer; lo STATO mutabile vive nel
    /// reader (ADR-IO 1).
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>>;
}

pub trait LayerReader {
    /// Schema effettivo del reader, autoritativo: il consumatore non lo inferisce
    /// (ADR-IO 6). Riflette la projection realmente applicata.
    fn contract(&self) -> &LayerContract;
    /// Pull-based con stato; `None` = fine dello stream.
    fn next_batch(&mut self) -> Result<Option<RecordBatch>>;
    /// Report di perdita (vuoto per i driver Lossless) — ADR-IO 5.
    fn loss_report(&self) -> LossReport {
        LossReport::default()
    }
}

pub trait FormatWriter {
    /// Valutazione preventiva prodotta da `create`; il `Published` finale la
    /// aggiorna con le perdite osservate durante la scrittura.
    fn fidelity_assessment(&self) -> FidelityAssessment {
        FidelityAssessment::unassessed(
            "writer non avvolto dal validatore comune: assessment non disponibile",
        )
    }
    /// Scrive un batch nel layer primario (`LayerId(0)`).
    fn write(&mut self, batch: &RecordBatch) -> Result<()>;
    /// Scrive un batch in uno specifico layer (multi-layer). Default: accetta solo
    /// `LayerId(0)` e delega a `write`; i driver multi-layer fanno override (ADR-IO 1:
    /// un dataset-writer coordina tutti i layer con un unico commit atomico).
    fn write_to_layer(&mut self, layer: LayerId, batch: &RecordBatch) -> Result<()> {
        if layer.0 != 0 {
            return Err(PlenoraIoError::Unsupported(
                "questo formato non supporta la scrittura multi-layer".to_owned(),
            ));
        }
        self.write(batch)
    }
    /// Publish atomico dell'intero dataset, solo a successo (D11, ADR-IO 2).
    fn finish(self: Box<Self>) -> Result<Published>;
}

/// Applica i limiti indipendenti dal formato a qualunque writer. I vincoli
/// specifici (WKB, vertici, dimensione fisica del dataset) restano nel driver.
pub fn with_write_limits(writer: Box<dyn FormatWriter>, limits: Limits) -> Box<dyn FormatWriter> {
    Box::new(LimitedWriter {
        inner: writer,
        driver: "writer",
        limits,
        rows: 0,
        failed: false,
        geometry_validation: None,
        planned_loss: LossReport::default(),
        cancellation: CancellationToken::new(),
        fidelity: FidelityAssessment::unassessed(
            "writer con soli limiti globali: assessment di formato non disponibile",
        ),
    })
}

/// Applica i limiti globali e verifica che i byte geometrici di ogni batch
/// rispettino sia il contratto dichiarato sia le capability del driver.
///
/// È una seconda guardia runtime: impedisce che un batch dichiarato XY contenga
/// in realtà WKB Z/M o EWKB e venga normalizzato silenziosamente dal driver.
fn geometry_contracts_for_validation(
    plan: &WritePlan,
) -> Result<Vec<Option<GeometryColumnContract>>> {
    plan.layers
        .iter()
        .map(|layer| -> Result<Option<GeometryColumnContract>> {
            if let Some(geometry) = &layer.contract.geometry {
                return Ok(Some(geometry.clone()));
            }
            let mut fields = layer
                .contract
                .schema
                .fields()
                .iter()
                .enumerate()
                .filter(|(_, field)| is_geometry_field(field));
            let Some((index, field)) = fields.next() else {
                return Ok(None);
            };
            if fields.next().is_some() {
                return Err(PlenoraIoError::Contract(format!(
                    "layer '{}': più colonne GeoArrow senza contratto geometrico esplicito",
                    layer.name
                )));
            }
            // Il costruttore stabilisce il default storico XY prima di leggere
            // i metadati legacy. Un valore esplicito, incluso `unknown`, lo
            // sostituisce e non viene mai degradato dopo il parsing (R3.4).
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(index as u32),
                field.name(),
                CrsResolution::Missing,
                field.is_nullable(),
            );
            read_geometry_contract_metadata(field, &mut geometry)?;
            Ok(Some(geometry))
        })
        .collect()
}

pub fn with_write_validation(
    writer: Box<dyn FormatWriter>,
    descriptor: &FormatDescriptor,
    plan: &WritePlan,
    limits: Limits,
    cancellation: CancellationToken,
) -> Result<Box<dyn FormatWriter>> {
    let geometry_support = descriptor
        .write_capabilities
        .as_ref()
        .map(|capabilities| capabilities.geometry);
    let layers = geometry_contracts_for_validation(plan)?;
    let planned_loss = planned_write_loss(descriptor, plan);
    let fidelity = assess_write_contract(descriptor, plan).with_loss_report(&planned_loss);
    Ok(Box::new(LimitedWriter {
        inner: writer,
        driver: descriptor.id,
        limits,
        rows: 0,
        failed: false,
        fidelity,
        planned_loss,
        cancellation,
        geometry_validation: geometry_support.map(|support| GeometryValidation {
            driver: descriptor.id,
            support,
            layers,
        }),
    }))
}

fn planned_write_loss(descriptor: &FormatDescriptor, plan: &WritePlan) -> LossReport {
    let mut loss = LossReport::default();
    let Some(capabilities) = descriptor.write_capabilities else {
        return loss;
    };

    for layer in &plan.layers {
        if capabilities.crs == CrsWriteSupport::None {
            if let Some(geometry) = &layer.contract.geometry {
                let has_crs_information =
                    !matches!(geometry.crs, CrsResolution::Missing) || geometry.srid.is_some();
                if has_crs_information {
                    loss.record("metadata CRS non rappresentati", 1);
                    loss.add_example(LossExample {
                        category: "metadata CRS non rappresentati".to_owned(),
                        context: format!("layer={} field={}", layer.name, geometry.name),
                    });
                }
            }
        }

        let geometry_name = layer
            .contract
            .geometry
            .as_ref()
            .map(|geometry| geometry.name.as_str());
        for field in layer.contract.schema.fields() {
            if geometry_name == Some(field.name().as_str()) || is_geometry_field(field) {
                continue;
            }
            let type_class = crate::capabilities::arrow_type_class(field.data_type());
            let unsupported_text_coercion = !capabilities.allowed_types.contains(&type_class)
                && matches!(
                    capabilities.type_coercion,
                    TypeCoercionPolicy::ExplicitText | TypeCoercionPolicy::LossReported
                );
            let kml_scalar_to_text = descriptor.id == "kml" && type_class != ArrowTypeClass::Utf8;
            let gpkg_type_normalization = descriptor.id == "gpkg"
                && !matches!(
                    field.data_type(),
                    DataType::Int64 | DataType::Float64 | DataType::Utf8 | DataType::Binary
                );
            if unsupported_text_coercion || kml_scalar_to_text || gpkg_type_normalization {
                loss.record("coercion tipo attributo", 1);
                loss.add_example(LossExample {
                    category: "coercion tipo attributo".to_owned(),
                    context: format!("layer={} field={}", layer.name, field.name()),
                });
            }
        }
    }
    loss
}

fn assess_write_contract(descriptor: &FormatDescriptor, plan: &WritePlan) -> FidelityAssessment {
    let mut assessment = FidelityAssessment::for_format(descriptor.id, descriptor.fidelity_class);
    let Some(capabilities) = descriptor.write_capabilities else {
        return assessment;
    };

    for layer in &plan.layers {
        let geometry_name = layer
            .contract
            .geometry
            .as_ref()
            .map(|geometry| geometry.name.as_str());
        for field in layer.contract.schema.fields() {
            let is_geometry = geometry_name == Some(field.name().as_str());
            if !is_geometry && capabilities.attributes == AttributeWriteSupport::LossReported {
                assessment.add_reason(
                    FidelityReasonCode::AttributeLoss,
                    format!(
                        "{}: attributo '{}' non nativo o loss-reported",
                        layer.name,
                        field.name()
                    ),
                );
            }
            if !capabilities
                .allowed_types
                .contains(&crate::capabilities::arrow_type_class(field.data_type()))
                && capabilities.type_coercion == TypeCoercionPolicy::LossReported
            {
                assessment.add_reason(
                    FidelityReasonCode::TypeCoercion,
                    format!(
                        "{}: tipo {:?} di '{}' richiede coercion",
                        layer.name,
                        field.data_type(),
                        field.name()
                    ),
                );
            }
            if field.is_nullable() && capabilities.nullability == NullabilitySupport::FormatDefined
            {
                assessment.add_reason(
                    FidelityReasonCode::NullabilityChanged,
                    format!(
                        "{}: nullability di '{}' definita dal formato",
                        layer.name,
                        field.name()
                    ),
                );
            }
        }

        if descriptor.id == "dxf"
            && layer.contract.geometry.as_ref().is_some_and(|geometry| {
                geometry.geometry_types.iter().any(|geometry_type| {
                    matches!(
                        geometry_type,
                        GeometryType::MultiPoint
                            | GeometryType::MultiLineString
                            | GeometryType::MultiPolygon
                            | GeometryType::GeometryCollection
                    )
                })
            })
        {
            assessment.add_reason(
                FidelityReasonCode::StructureChanged,
                format!("{}: geometrie multipart esplose in entità DXF", layer.name),
            );
        }
    }
    assessment
}

struct GeometryValidation {
    driver: &'static str,
    support: GeometryWriteSupport,
    layers: Vec<Option<GeometryColumnContract>>,
}

struct LimitedWriter {
    inner: Box<dyn FormatWriter>,
    driver: &'static str,
    limits: Limits,
    rows: usize,
    failed: bool,
    geometry_validation: Option<GeometryValidation>,
    fidelity: FidelityAssessment,
    planned_loss: LossReport,
    cancellation: CancellationToken,
}

impl LimitedWriter {
    fn account(&mut self, layer: usize, batch: &RecordBatch) -> Result<()> {
        if batch.num_columns() > self.limits.max_columns {
            return Err(PlenoraIoError::LimitExceeded(format!(
                "batch con {} colonne oltre il limite di {}",
                batch.num_columns(),
                self.limits.max_columns
            )));
        }
        self.rows = self.rows.checked_add(batch.num_rows()).ok_or_else(|| {
            PlenoraIoError::LimitExceeded("overflow nel conteggio delle righe".to_owned())
        })?;
        if self.rows > self.limits.max_rows {
            return Err(PlenoraIoError::LimitExceeded(format!(
                "{} righe oltre il limite di {}",
                self.rows, self.limits.max_rows
            )));
        }
        if let Some(validation) = &self.geometry_validation {
            validate_geometry_batch(
                validation.driver,
                validation.support,
                validation.layers.get(layer).ok_or_else(|| {
                    PlenoraIoError::capability(
                        validation.driver,
                        None,
                        CapabilityReason::MultipleLayers,
                        format!("layer runtime {layer} fuori dal WritePlan"),
                    )
                })?,
                batch,
                &self.limits,
            )?;
        }
        Ok(())
    }
}

impl FormatWriter for LimitedWriter {
    fn fidelity_assessment(&self) -> FidelityAssessment {
        self.fidelity.clone()
    }

    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        check_cancelled(&self.cancellation, ErrorPhase::Write)?;
        if self.failed {
            return Err(PlenoraIoError::format(
                self.driver,
                "writer invalidato da un precedente errore di scrittura",
            )
            .during(plenora_io_model::ErrorPhase::Write));
        }
        let result = self
            .account(0, batch)
            .and_then(|()| self.inner.write(batch));
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn write_to_layer(&mut self, layer: LayerId, batch: &RecordBatch) -> Result<()> {
        check_cancelled(&self.cancellation, ErrorPhase::Write)?;
        if self.failed {
            return Err(PlenoraIoError::format(
                self.driver,
                "writer invalidato da un precedente errore di scrittura",
            )
            .during(plenora_io_model::ErrorPhase::Write));
        }
        let result = self
            .account(layer.0 as usize, batch)
            .and_then(|()| self.inner.write_to_layer(layer, batch));
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn finish(self: Box<Self>) -> Result<Published> {
        check_cancelled(&self.cancellation, ErrorPhase::Finalize)?;
        if self.failed {
            return Err(PlenoraIoError::format(
                self.driver,
                "finish vietato dopo un errore di scrittura",
            )
            .during(plenora_io_model::ErrorPhase::Finalize));
        }
        let mut published = self.inner.finish()?;
        published.loss.merge(&self.planned_loss);
        published.fidelity = self.fidelity.with_loss_report(&published.loss);
        Ok(published)
    }
}

fn geometry_violation(
    driver: &'static str,
    field: &str,
    reason: CapabilityReason,
    detail: impl Into<String>,
) -> PlenoraIoError {
    PlenoraIoError::capability(driver, Some(field.to_owned()), reason, detail)
}

fn geometry_nodes_match(
    geometry: &WkbGeometry,
    dimensions: CoordinateDimensions,
    allow_srid: bool,
) -> bool {
    if geometry.dimensions != dimensions || (!allow_srid && geometry.srid.is_some()) {
        return false;
    }
    match &geometry.value {
        WkbValue::MultiPoint(values)
        | WkbValue::MultiLineString(values)
        | WkbValue::MultiPolygon(values)
        | WkbValue::GeometryCollection(values) => values
            .iter()
            .all(|value| geometry_nodes_match(value, dimensions, allow_srid)),
        WkbValue::Point(_) | WkbValue::LineString(_) | WkbValue::Polygon(_) => true,
    }
}

fn validate_decoded_geometry(
    driver: &'static str,
    support: GeometryWriteSupport,
    contract: &GeometryColumnContract,
    geometry: &WkbGeometry,
) -> Result<()> {
    let actual_dimensions = geometry.dimensions;
    if contract.dimensions != CoordinateDimensions::Unknown
        && contract.dimensions != actual_dimensions
    {
        return Err(geometry_violation(
            driver,
            &contract.name,
            CapabilityReason::CoordinateDimensions,
            format!(
                "payload {:?} diverso dalle dimensioni dichiarate {:?}",
                actual_dimensions, contract.dimensions
            ),
        ));
    }
    if !support.dimensions.contains(&actual_dimensions) {
        return Err(geometry_violation(
            driver,
            &contract.name,
            CapabilityReason::CoordinateDimensions,
            format!("payload {:?} non supportato dal driver", actual_dimensions),
        ));
    }

    let allow_srid = contract.encoding == GeometryEncoding::Ewkb;
    if !geometry_nodes_match(geometry, actual_dimensions, allow_srid) {
        return Err(geometry_violation(
            driver,
            &contract.name,
            if allow_srid {
                CapabilityReason::CoordinateDimensions
            } else {
                CapabilityReason::GeometryEncoding
            },
            "componenti WKB con dimensioni incoerenti o SRID EWKB non dichiarato",
        ));
    }
    if contract.encoding == GeometryEncoding::Ewkb && geometry.srid != contract.srid {
        return Err(geometry_violation(
            driver,
            &contract.name,
            CapabilityReason::GeometryEncoding,
            format!(
                "SRID del payload {:?} diverso da quello dichiarato {:?}",
                geometry.srid, contract.srid
            ),
        ));
    }
    if !contract.geometry_types.is_empty()
        && !contract.geometry_types.contains(&geometry.geometry_type())
    {
        return Err(geometry_violation(
            driver,
            &contract.name,
            CapabilityReason::MixedGeometry,
            format!(
                "tipo {:?} assente dai tipi geometrici dichiarati",
                geometry.geometry_type()
            ),
        ));
    }
    Ok(())
}

fn validate_geometry_batch(
    driver: &'static str,
    support: GeometryWriteSupport,
    contract: &Option<GeometryColumnContract>,
    batch: &RecordBatch,
    limits: &Limits,
) -> Result<()> {
    let Some(contract) = contract else {
        return Ok(());
    };
    let index = batch
        .schema()
        .fields()
        .iter()
        .position(|field| field.name() == &contract.name)
        .ok_or_else(|| {
            geometry_violation(
                driver,
                &contract.name,
                CapabilityReason::GeometryNotSupported,
                "colonna geometrica dichiarata assente dal batch",
            )
        })?;
    let array = batch.column(index);
    let wkb_limits = limits.effective_wkb();

    let validate_value = |row: usize, bytes: Option<&[u8]>| -> Result<()> {
        let Some(bytes) = bytes else {
            if !contract.nullable {
                return Err(geometry_violation(
                    driver,
                    &contract.name,
                    CapabilityReason::Nullability,
                    format!("geometria nulla alla riga {row} in colonna non-nullable"),
                ));
            }
            return Ok(());
        };
        let geometry = decode_wkb(bytes, &wkb_limits)?;
        validate_decoded_geometry(driver, support, contract, &geometry)
    };

    if let Some(values) = array.as_any().downcast_ref::<BinaryArray>() {
        for row in 0..values.len() {
            validate_value(
                row,
                if values.is_null(row) {
                    None
                } else {
                    Some(values.value(row))
                },
            )?;
        }
        return Ok(());
    }
    if let Some(values) = array.as_any().downcast_ref::<LargeBinaryArray>() {
        for row in 0..values.len() {
            validate_value(
                row,
                if values.is_null(row) {
                    None
                } else {
                    Some(values.value(row))
                },
            )?;
        }
        return Ok(());
    }
    Err(geometry_violation(
        driver,
        &contract.name,
        CapabilityReason::GeometryEncoding,
        "colonna geometrica runtime non Binary/LargeBinary",
    ))
}

pub struct Published {
    pub bytes: u64,
    pub loss: LossReport,
    /// Valutazione specifica della scrittura conclusa (ADR-IO 5).
    pub fidelity: FidelityAssessment,
    /// Esito di durabilità del publish (ADR-IO 2).
    pub outcome: crate::publish::PublishOutcome,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use arrow_array::{BinaryArray, Int64Array};
    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_model::contract::{CoordinateDimensions, FieldId, GeometryColumnContract};
    use plenora_io_model::crs::CrsResolution;
    use plenora_io_model::geometry::{
        ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, PLENORA_DIMENSIONS_KEY,
    };
    use plenora_io_model::wkb::{encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};

    use super::*;
    use crate::descriptor::WKB_XY_GEOMETRY;

    #[test]
    fn periodic_cancellation_has_a_bounded_check_interval() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            check_cancelled_periodically(&token, ErrorPhase::Read, 0),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Cancelled
        ));
        assert!(check_cancelled_periodically(&token, ErrorPhase::Read, 1).is_ok());
        assert!(matches!(
            check_cancelled_periodically(
                &token,
                ErrorPhase::Read,
                CANCELLATION_CHECK_INTERVAL
            ),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Cancelled
        ));
    }

    struct FinishTrackingWriter {
        finished: Arc<AtomicBool>,
    }

    impl FormatWriter for FinishTrackingWriter {
        fn write(&mut self, _batch: &RecordBatch) -> Result<()> {
            Ok(())
        }

        fn finish(self: Box<Self>) -> Result<Published> {
            self.finished.store(true, Ordering::SeqCst);
            Ok(Published {
                bytes: 0,
                loss: LossReport::default(),
                fidelity: FidelityAssessment::lossless(),
                outcome: crate::publish::PublishOutcome::Published,
            })
        }
    }

    #[test]
    fn failed_write_poisons_writer_and_prevents_finish() {
        let finished = Arc::new(AtomicBool::new(false));
        let limits = Limits {
            max_rows: 0,
            ..Limits::default()
        };
        let mut writer = with_write_limits(
            Box::new(FinishTrackingWriter {
                finished: finished.clone(),
            }),
            limits,
        );
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Int64,
            false,
        )]));
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![1]))]).unwrap();

        assert!(matches!(
            writer.write(&batch),
            Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded
        ));
        assert!(matches!(
            writer.write(&batch),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Format
        ));
        assert!(matches!(
            writer.finish(),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Format
        ));
        assert!(!finished.load(Ordering::SeqCst));
    }

    #[test]
    fn source_size_is_checked_before_parsing() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&[0_u8; 8]).unwrap();
        let limits = Limits {
            max_input_bytes: 7,
            ..Limits::default()
        };
        let result = Source::Path(file.path().to_owned())
            .into_path_checked(&limits, &CancellationToken::new());
        assert!(matches!(
            result,
            Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded
        ));
    }

    #[test]
    fn cancelled_source_is_rejected_before_filesystem_probe() {
        let token = CancellationToken::new();
        token.cancel();
        let result = Source::Path(std::path::PathBuf::from("not-observed"))
            .into_path_checked(&Limits::default(), &token);
        assert!(matches!(
            result,
            Err(error)
                if error.code == plenora_io_model::IoErrorCode::Cancelled
                    && error.phase == ErrorPhase::Probe
        ));
    }

    #[test]
    fn cancellation_before_finish_never_publishes() {
        let finished = Arc::new(AtomicBool::new(false));
        let token = CancellationToken::new();
        let writer: Box<dyn FormatWriter> = Box::new(LimitedWriter {
            inner: Box::new(FinishTrackingWriter {
                finished: finished.clone(),
            }),
            driver: "test",
            limits: Limits::default(),
            rows: 0,
            failed: false,
            geometry_validation: None,
            fidelity: FidelityAssessment::lossless(),
            planned_loss: LossReport::default(),
            cancellation: token.clone(),
        });
        token.cancel();

        assert!(matches!(
            writer.finish(),
            Err(error)
                if error.code == plenora_io_model::IoErrorCode::Cancelled
                    && error.phase == ErrorPhase::Finalize
        ));
        assert!(!finished.load(Ordering::SeqCst));
    }

    struct TestReader {
        layer: LayerContract,
        batches: usize,
        fail: bool,
    }

    impl LayerReader for TestReader {
        fn contract(&self) -> &LayerContract {
            &self.layer
        }

        fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            if self.fail {
                self.fail = false;
                return Err(PlenoraIoError::Contract("errore terminale".to_owned()));
            }
            if self.batches == 0 {
                return Ok(None);
            }
            self.batches -= 1;
            Ok(Some(RecordBatch::new_empty(Arc::new(Schema::empty()))))
        }
    }

    fn test_reader(batches: usize, fail: bool) -> Box<dyn LayerReader> {
        Box::new(TestReader {
            layer: test_layer(),
            batches,
            fail,
        })
    }

    fn test_layer() -> LayerContract {
        LayerContract {
            id: LayerId(0),
            name: "layer".to_owned(),
            contract: plenora_io_model::contract::DataContract {
                schema: Arc::new(Schema::empty()),
                geometry: None,
            },
        }
    }

    fn fixed_batch_reader(values: Vec<i64>) -> Box<dyn LayerReader> {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Int64,
            false,
        )]));
        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(values))]).unwrap();
        Box::new(FixedBatchReader {
            layer: LayerContract {
                id: LayerId(0),
                name: "layer".to_owned(),
                contract: plenora_io_model::contract::DataContract {
                    schema,
                    geometry: None,
                },
            },
            batch: Some(batch),
        })
    }

    struct FixedBatchReader {
        layer: LayerContract,
        batch: Option<RecordBatch>,
    }

    impl LayerReader for FixedBatchReader {
        fn contract(&self) -> &LayerContract {
            &self.layer
        }

        fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            Ok(self.batch.take())
        }
    }

    #[test]
    fn batch_target_slices_without_reordering_and_releases_gate_at_eof() {
        let gate = SingleReaderGate::new("test");
        let inner = gate
            .open(LayerId(0), || Ok(fixed_batch_reader(vec![0, 1, 2, 3, 4])))
            .unwrap();
        let mut reader = with_batch_target(
            inner,
            BatchTarget {
                target_bytes: 16,
                max_rows: 100,
            },
            CancellationToken::new(),
        );
        assert!(matches!(
            gate.open(LayerId(0), || Ok(test_reader(1, false))),
            Err(error) if error.code == plenora_io_model::IoErrorCode::ReaderBusy
        ));

        let mut sizes = Vec::new();
        let mut values = Vec::new();
        while let Some(batch) = reader.next_batch().unwrap() {
            sizes.push(batch.num_rows());
            values.extend_from_slice(
                batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .values(),
            );
        }
        assert_eq!(sizes, vec![2, 2, 1]);
        assert_eq!(values, vec![0, 1, 2, 3, 4]);
        assert!(gate.open(LayerId(0), || Ok(test_reader(1, false))).is_ok());
    }

    #[test]
    fn single_reader_gate_releases_on_drop_eof_and_error() {
        let gate = SingleReaderGate::new("test");
        let first = gate.open(LayerId(0), || Ok(test_reader(1, false))).unwrap();
        assert!(matches!(
            gate.open(LayerId(0), || Ok(test_reader(1, false))),
            Err(error)
                if error.code == plenora_io_model::IoErrorCode::ReaderBusy
                    && error.driver.as_deref() == Some("test")
        ));

        drop(first);
        assert!(gate
            .open(LayerId(0), || {
                Err(PlenoraIoError::Contract("costruzione fallita".to_owned()))
            })
            .is_err());
        let mut exhausted = gate.open(LayerId(0), || Ok(test_reader(1, false))).unwrap();
        assert!(exhausted.next_batch().unwrap().is_some());
        assert!(exhausted.next_batch().unwrap().is_none());
        let after_eof = gate.open(LayerId(0), || Ok(test_reader(1, false))).unwrap();
        drop(after_eof);

        let mut failed = gate.open(LayerId(0), || Ok(test_reader(0, true))).unwrap();
        assert!(failed.next_batch().is_err());
        assert!(gate.open(LayerId(0), || Ok(test_reader(1, false))).is_ok());
    }

    #[test]
    fn cancelled_reader_releases_single_reader_lease() {
        let gate = SingleReaderGate::new("test");
        let inner = gate.open(LayerId(0), || Ok(test_reader(1, false))).unwrap();
        let token = CancellationToken::new();
        let mut reader = with_cancellation(inner, token.clone());
        token.cancel();

        assert!(matches!(
            reader.next_batch(),
            Err(error)
                if error.code == plenora_io_model::IoErrorCode::Cancelled
                    && error.phase == ErrorPhase::Read
        ));
        assert!(gate.open(LayerId(0), || Ok(test_reader(1, false))).is_ok());
    }

    fn geometry_batch(bytes: Option<&[u8]>) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "geometry",
            DataType::Binary,
            true,
        )]));
        let geometry = BinaryArray::from(vec![bytes]);
        RecordBatch::try_new(schema, vec![Arc::new(geometry)]).unwrap()
    }

    fn xy_contract(nullable: bool) -> GeometryColumnContract {
        GeometryColumnContract::wkb_xy(FieldId(0), "geometry", CrsResolution::Missing, nullable)
    }

    #[test]
    fn runtime_geometry_validation_rejects_hidden_z_payload() {
        let xyz = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: Some(3.0),
                m: None,
            }),
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        };
        let bytes = encode_wkb(&xyz, WkbFlavor::Iso).unwrap();
        let result = validate_geometry_batch(
            "test",
            WKB_XY_GEOMETRY,
            &Some(xy_contract(true)),
            &geometry_batch(Some(&bytes)),
            &Limits::default(),
        );
        assert!(matches!(
            result,
            Err(error)
                if error.capability_reason == Some(CapabilityReason::CoordinateDimensions)
        ));
    }

    #[test]
    fn runtime_geometry_validation_rejects_undeclared_ewkb_srid() {
        let ewkb = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xy,
            srid: Some(4326),
        };
        let bytes = encode_wkb(&ewkb, WkbFlavor::Ewkb).unwrap();
        let result = validate_geometry_batch(
            "test",
            WKB_XY_GEOMETRY,
            &Some(xy_contract(true)),
            &geometry_batch(Some(&bytes)),
            &Limits::default(),
        );
        assert!(matches!(
            result,
            Err(error)
                if error.capability_reason == Some(CapabilityReason::GeometryEncoding)
        ));
    }

    #[test]
    fn runtime_geometry_validation_enforces_nullability() {
        let result = validate_geometry_batch(
            "test",
            WKB_XY_GEOMETRY,
            &Some(xy_contract(false)),
            &geometry_batch(None),
            &Limits::default(),
        );
        assert!(matches!(
            result,
            Err(error) if error.capability_reason == Some(CapabilityReason::Nullability)
        ));
    }

    fn geoarrow_field(name: &str, dimensions: Option<&str>) -> Field {
        let mut metadata = HashMap::from([(
            ARROW_EXTENSION_NAME_KEY.to_owned(),
            GEOARROW_WKB_EXTENSION.to_owned(),
        )]);
        if let Some(dimensions) = dimensions {
            metadata.insert(PLENORA_DIMENSIONS_KEY.to_owned(), dimensions.to_owned());
        }
        Field::new(name, DataType::Binary, true).with_metadata(metadata)
    }

    fn legacy_plan(fields: Vec<Field>) -> WritePlan {
        WritePlan {
            layers: vec![crate::request::WriteLayer {
                name: "layer".to_owned(),
                contract: plenora_io_model::contract::DataContract {
                    schema: Arc::new(Schema::new(fields)),
                    geometry: None,
                },
            }],
        }
    }

    #[test]
    fn legacy_geometry_defaults_xy_only_when_dimensions_are_absent() {
        let absent =
            geometry_contracts_for_validation(&legacy_plan(vec![geoarrow_field("geometry", None)]))
                .unwrap();
        let explicit_unknown =
            geometry_contracts_for_validation(&legacy_plan(vec![geoarrow_field(
                "geometry",
                Some("unknown"),
            )]))
            .unwrap();

        assert_eq!(
            absent[0].as_ref().unwrap().dimensions,
            CoordinateDimensions::Xy
        );
        assert_eq!(
            explicit_unknown[0].as_ref().unwrap().dimensions,
            CoordinateDimensions::Unknown
        );
    }

    #[test]
    fn ambiguous_or_invalid_legacy_geometry_metadata_is_rejected() {
        let ambiguous = legacy_plan(vec![
            geoarrow_field("geometry_a", None),
            geoarrow_field("geometry_b", None),
        ]);
        let invalid = legacy_plan(vec![geoarrow_field("geometry", Some("future"))]);

        assert!(matches!(
            geometry_contracts_for_validation(&ambiguous),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
        ));
        assert!(matches!(
            geometry_contracts_for_validation(&invalid),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
        ));
    }
}
