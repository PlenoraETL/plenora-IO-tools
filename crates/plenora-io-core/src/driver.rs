//! Il confine plug-in: `FormatDriver` + handle/reader/writer (ADR-IO 1).

use std::collections::BTreeMap;
use std::path::PathBuf;

use arrow_array::{Array, BinaryArray, LargeBinaryArray, RecordBatch};
use plenora_core::contract::{
    CoordinateDimensions, FieldId, GeometryColumnContract, GeometryEncoding, GeometryType,
    LayerContract, LayerId,
};
use plenora_core::crs::CrsResolution;
use plenora_core::geometry::{is_geometry_field, read_geometry_contract_metadata};
use plenora_core::limits::Limits;
use plenora_core::wkb::{decode_wkb, WkbGeometry, WkbValue};
use plenora_core::{CapabilityReason, PlenoraError, Result};

use crate::descriptor::{
    AttributeWriteSupport, FormatDescriptor, GeometryWriteSupport, NullabilitySupport,
    TypeCoercionPolicy,
};
use crate::loss::{FidelityAssessment, FidelityReasonCode, LossReport};
use crate::request::{ReadRequest, WritePlan};

/// Sorgente di lettura (scheletro Fase 0).
pub enum Source {
    Path(PathBuf),
}

impl Source {
    /// Risolve la sorgente e applica il limite complessivo prima che un parser
    /// possa materializzarla. Le directory-dataset sono conteggiate senza
    /// seguire symlink.
    pub fn into_path_checked(self, limits: &Limits) -> Result<PathBuf> {
        let Self::Path(path) = self;
        let mut total = 0_u64;
        let mut pending = vec![path.clone()];
        while let Some(candidate) = pending.pop() {
            let metadata = std::fs::symlink_metadata(&candidate)?;
            if metadata.file_type().is_symlink() {
                return Err(PlenoraError::Unsupported(format!(
                    "symlink non ammesso nella sorgente: {}",
                    candidate.display()
                )));
            }
            if metadata.is_dir() {
                for entry in std::fs::read_dir(&candidate)? {
                    pending.push(entry?.path());
                }
            } else if metadata.is_file() {
                total = total.checked_add(metadata.len()).ok_or_else(|| {
                    PlenoraError::LimitExceeded("overflow nel conteggio dell'input".to_owned())
                })?;
                if total > limits.max_input_bytes {
                    return Err(PlenoraError::LimitExceeded(format!(
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
}

#[derive(Default)]
pub struct WriteOptions {
    /// Profilo `DurableAtomicPublish` (fsync) invece di `AtomicPublish` — ADR-IO 2.
    pub durable: bool,
    /// Knob specifici del driver.
    pub format_options: BTreeMap<String, String>,
    /// Limiti condivisi del bordo I/O.
    pub limits: Limits,
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

pub trait OpenDatasetHandle {
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
            return Err(PlenoraError::Unsupported(
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
        limits,
        rows: 0,
        geometry_validation: None,
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
pub fn with_write_validation(
    writer: Box<dyn FormatWriter>,
    descriptor: &FormatDescriptor,
    plan: &WritePlan,
    limits: Limits,
) -> Box<dyn FormatWriter> {
    let geometry_support = descriptor
        .write_capabilities
        .as_ref()
        .map(|capabilities| capabilities.geometry);
    let layers = plan
        .layers
        .iter()
        .map(|layer| {
            layer.contract.geometry.clone().or_else(|| {
                let mut fields = layer
                    .contract
                    .schema
                    .fields()
                    .iter()
                    .enumerate()
                    .filter(|(_, field)| is_geometry_field(field));
                let (index, field) = fields.next()?;
                if fields.next().is_some() {
                    return None;
                }
                // Compatibilità con i contratti v1 che marcavano solo il campo
                // GeoArrow: in assenza dei nuovi metadati, WKB XY è il default
                // storico e viene comunque verificato contro i byte runtime.
                let mut geometry = GeometryColumnContract::wkb_xy(
                    FieldId(index as u32),
                    field.name(),
                    CrsResolution::Missing,
                    field.is_nullable(),
                );
                read_geometry_contract_metadata(field, &mut geometry);
                if geometry.dimensions == CoordinateDimensions::Unknown {
                    geometry.dimensions = CoordinateDimensions::Xy;
                }
                Some(geometry)
            })
        })
        .collect();
    Box::new(LimitedWriter {
        inner: writer,
        limits,
        rows: 0,
        fidelity: assess_write_contract(descriptor, plan),
        geometry_validation: geometry_support.map(|support| GeometryValidation {
            driver: descriptor.id,
            support,
            layers,
        }),
    })
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
    limits: Limits,
    rows: usize,
    geometry_validation: Option<GeometryValidation>,
    fidelity: FidelityAssessment,
}

impl LimitedWriter {
    fn account(&mut self, layer: usize, batch: &RecordBatch) -> Result<()> {
        if batch.num_columns() > self.limits.max_columns {
            return Err(PlenoraError::LimitExceeded(format!(
                "batch con {} colonne oltre il limite di {}",
                batch.num_columns(),
                self.limits.max_columns
            )));
        }
        self.rows = self.rows.checked_add(batch.num_rows()).ok_or_else(|| {
            PlenoraError::LimitExceeded("overflow nel conteggio delle righe".to_owned())
        })?;
        if self.rows > self.limits.max_rows {
            return Err(PlenoraError::LimitExceeded(format!(
                "{} righe oltre il limite di {}",
                self.rows, self.limits.max_rows
            )));
        }
        if let Some(validation) = &self.geometry_validation {
            validate_geometry_batch(
                validation.driver,
                validation.support,
                validation
                    .layers
                    .get(layer)
                    .ok_or_else(|| PlenoraError::Capability {
                        driver: validation.driver,
                        field: None,
                        reason: CapabilityReason::MultipleLayers,
                        detail: format!("layer runtime {layer} fuori dal WritePlan"),
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
        self.account(0, batch)?;
        self.inner.write(batch)
    }

    fn write_to_layer(&mut self, layer: LayerId, batch: &RecordBatch) -> Result<()> {
        self.account(layer.0 as usize, batch)?;
        self.inner.write_to_layer(layer, batch)
    }

    fn finish(self: Box<Self>) -> Result<Published> {
        let mut published = self.inner.finish()?;
        published.fidelity = self.fidelity.with_loss_report(&published.loss);
        Ok(published)
    }
}

fn geometry_violation(
    driver: &'static str,
    field: &str,
    reason: CapabilityReason,
    detail: impl Into<String>,
) -> PlenoraError {
    PlenoraError::Capability {
        driver,
        field: Some(field.to_owned()),
        reason,
        detail: detail.into(),
    }
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
    use std::io::Write;
    use std::sync::Arc;

    use arrow_array::BinaryArray;
    use arrow_schema::{DataType, Field, Schema};
    use plenora_core::contract::{CoordinateDimensions, FieldId, GeometryColumnContract};
    use plenora_core::crs::CrsResolution;
    use plenora_core::wkb::{encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};

    use super::*;
    use crate::descriptor::WKB_XY_GEOMETRY;

    #[test]
    fn source_size_is_checked_before_parsing() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&[0_u8; 8]).unwrap();
        let limits = Limits {
            max_input_bytes: 7,
            ..Limits::default()
        };
        let result = Source::Path(file.path().to_owned()).into_path_checked(&limits);
        assert!(matches!(result, Err(PlenoraError::LimitExceeded(_))));
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
            Err(PlenoraError::Capability {
                reason: CapabilityReason::CoordinateDimensions,
                ..
            })
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
            Err(PlenoraError::Capability {
                reason: CapabilityReason::GeometryEncoding,
                ..
            })
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
            Err(PlenoraError::Capability {
                reason: CapabilityReason::Nullability,
                ..
            })
        ));
    }
}
