//! driver-ipc — Arrow IPC (`.arrow`) ⇄ RecordBatch. **Pass-through nativo**:
//! l'IPC È già Arrow, quindi schema (inclusi i metadati `geoarrow.wkb` + `crs`) e
//! buffer passano SENZA conversione — Lossless, zero decode/encode WKB, streaming
//! reale (il `FileReader` è un iteratore pull, nessun thread). È il formato di
//! interscambio canonico fra plenora-IO-tools e plenora-data-tools.
#![forbid(unsafe_code)]

use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_ipc::reader::FileReader;
use arrow_ipc::writer::FileWriter;
use arrow_schema::{Schema, SchemaRef};

use plenora_io_core::descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, ReadMode, ReaderConcurrency, Runtime,
    WriteMode,
};
use plenora_io_core::driver::{
    FormatDriver, FormatWriter, LayerReader, OpenDatasetHandle, Published, ReadOptions, Sink,
    Source, WriteOptions,
};
use plenora_io_core::loss::LossReport;
use plenora_io_core::publish::StagedFile;
use plenora_io_core::request::ReadRequest;
use plenora_io_core::{
    validate_write, with_write_validation, AttributeWriteSupport, CrsRepresentationCapabilities,
    CrsRepresentationState, CrsWriteSupport, FormatWriteCapabilities, NullabilitySupport,
    TypeCoercionPolicy, WritePlan, ALL_ARROW_TYPES, UTF8_FIELD_NAMES,
    WKB_EWKB_PASSTHROUGH_GEOMETRY,
};
use plenora_io_model::contract::{
    DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId,
};
use plenora_io_model::crs::{CrsKind, CrsResolution, ResolvedCrs};
use plenora_io_model::geometry::{
    is_geometry_field, read_geometry_contract_metadata, validate_contract_version,
    with_contract_version, with_geometry_contract_metadata, GEO_CRS_KEY,
};
use plenora_io_model::{PlenoraIoError, Result};

fn err(reason: impl Into<String>) -> PlenoraIoError {
    PlenoraIoError::format("ipc", reason)
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "ipc",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingSequential,
    read_determinism: plenora_io_core::DeterminismLevel::Semantic,
    write_mode: Some(WriteMode::Streaming),
    write_determinism: Some(plenora_io_core::DeterminismLevel::Semantic),
    multi_layer: false,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::MultipleIndependentReaders,
    projection_support: plenora_io_core::ProjectionSupport::Exact,
    predicate_pruning_support: plenora_io_core::PredicatePruningSupport::None,
    spatial_pruning_support: plenora_io_core::SpatialPruningSupport::None,
    crs_handling: CrsHandling::Embedded, // il CRS viaggia nei metadati del campo
    fidelity_class: Fidelity::Lossless,
    runtime: Runtime::PureRust,
    write_capabilities: Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: ALL_ARROW_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_EWKB_PASSTHROUGH_GEOMETRY,
        crs: CrsWriteSupport::EmbeddedOptional,
        crs_representations: CrsRepresentationCapabilities::new(
            CrsRepresentationState::Preserved,
            CrsRepresentationState::Preserved,
            CrsRepresentationState::Preserved,
        ),
        nullability: NullabilitySupport::Preserve,
        multi_layer: false,
    }),
    semantic_version: 1,
    driver_version: 3,
    descriptor_version: 7,
};

pub struct IpcDriver;

impl FormatDriver for IpcDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = source.into_path_checked(&opts.limits, &opts.cancellation)?;
        let reader = FileReader::try_new(File::open(&path)?, None)
            .map_err(|e| err(format!("Arrow IPC non valido: {e}")))?;
        let schema = reader.schema();
        validate_contract_version(schema.as_ref())?;
        let mut geometry_fields = schema
            .fields()
            .iter()
            .enumerate()
            .filter(|(_, field)| is_geometry_field(field));
        let geometry = match geometry_fields.next() {
            None => None,
            Some((i, _)) => {
                if geometry_fields.next().is_some() {
                    return Err(PlenoraIoError::Contract(
                        "Arrow IPC contiene più colonne GeoArrow nel contratto v1".to_owned(),
                    ));
                }
                let f = schema.field(i);
                let crs = match f.metadata().get(GEO_CRS_KEY).cloned() {
                    Some(id) => {
                        let kind = if id.eq_ignore_ascii_case("OGC:CRS84")
                            || id.eq_ignore_ascii_case("EPSG:4326")
                        {
                            CrsKind::Geographic
                        } else {
                            CrsKind::Unknown
                        };
                        CrsResolution::resolved(ResolvedCrs::new(Some(id), kind, None))
                    }
                    None => CrsResolution::Missing,
                };
                let mut contract = GeometryColumnContract::wkb_passthrough(
                    FieldId(i as u32),
                    f.name(),
                    crs,
                    f.is_nullable(),
                );
                read_geometry_contract_metadata(f, &mut contract)?;
                Some(contract)
            }
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(Box::new(IpcDataset {
            path,
            layers: vec![LayerContract {
                id: LayerId(0),
                name,
                contract: DataContract::new(schema, geometry),
            }],
        }))
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        validate_write(self.descriptor(), plan, &opts.limits)?;
        let Sink::Path(path) = sink;
        if path.exists() {
            return Err(PlenoraIoError::OutputExists(path.display().to_string()));
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("arrow"))
        {
            return Err(PlenoraIoError::Unsupported(
                "l'output deve avere estensione .arrow".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::Unsupported(
                "Arrow IPC: un solo layer per file".to_owned(),
            ));
        }
        let layer = &plan.layers[0].contract;
        let fields = layer
            .schema
            .fields()
            .iter()
            .map(|field| {
                layer
                    .geometry
                    .as_ref()
                    .filter(|geometry| geometry.name.as_str() == field.name().as_str())
                    .map(|geometry| with_geometry_contract_metadata(field, geometry))
                    .unwrap_or_else(|| field.as_ref().clone())
            })
            .collect::<Vec<_>>();
        let schema = with_contract_version(Arc::new(arrow_schema::Schema::new_with_metadata(
            fields,
            layer.schema.metadata().clone(),
        )));
        let staging = StagedFile::new(&path, opts.durable, opts.limits.max_output_bytes)?;
        let writer = FileWriter::try_new(BufWriter::new(staging.reopen()?), &schema)
            .map_err(|e| err(format!("writer IPC: {e}")))?;
        with_write_validation(
            Box::new(IpcWriter {
                staging,
                writer: Some(writer),
                schema,
            }),
            self.descriptor(),
            plan,
            opts.limits,
            opts.cancellation.clone(),
        )
    }
}

struct IpcDataset {
    path: PathBuf,
    layers: Vec<LayerContract>,
}

impl OpenDatasetHandle for IpcDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(DESCRIPTOR.id, DESCRIPTOR.fidelity_class)
    }
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
        if request.layer != self.layers[0].id {
            return Err(err(format!("layer {} inesistente", request.layer.0)));
        }

        let source_layer = &self.layers[0];
        let (projection, layer) = match &request.projected_fields {
            None => (None, source_layer.clone()),
            Some(field_ids) => {
                let mut indices = Vec::new();
                for field_id in field_ids {
                    let index = field_id.0 as usize;
                    if index >= source_layer.contract.schema.fields().len() {
                        if request.projection_mode == plenora_io_core::ProjectionMode::Required {
                            return Err(PlenoraIoError::Contract(format!(
                                "projection Required: field id {} fuori range",
                                field_id.0
                            )));
                        }
                        continue;
                    }
                    if !indices.contains(&index) {
                        indices.push(index);
                    }
                }
                indices.sort_unstable();
                let fields = indices
                    .iter()
                    .map(|&index| source_layer.contract.schema.field(index).as_ref().clone())
                    .collect::<Vec<_>>();
                let schema = Arc::new(Schema::new_with_metadata(
                    fields,
                    source_layer.contract.schema.metadata().clone(),
                ));
                let geometry = source_layer.contract.geometry.clone().and_then(|geometry| {
                    schema
                        .index_of(&geometry.name)
                        .ok()
                        .map(|index| GeometryColumnContract {
                            field_id: FieldId(index as u32),
                            ..geometry
                        })
                });
                (
                    Some(indices),
                    LayerContract {
                        id: source_layer.id,
                        name: source_layer.name.clone(),
                        contract: DataContract::new(schema, geometry),
                    },
                )
            }
        };
        let reader = FileReader::try_new(File::open(&self.path)?, projection)
            .map_err(|e| err(format!("Arrow IPC non valido: {e}")))?;
        Ok(plenora_io_core::with_batch_target(
            Box::new(IpcReader { reader, layer }),
            request.batch_target,
            request.cancellation.clone(),
        ))
    }
}

struct IpcReader {
    reader: FileReader<File>,
    layer: LayerContract,
}

impl LayerReader for IpcReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }
    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        match self.reader.next() {
            None => Ok(None),
            Some(Ok(b)) => Ok(Some(b)),
            Some(Err(e)) => Err(err(format!("batch IPC: {e}"))),
        }
    }
}

struct IpcWriter {
    staging: StagedFile,
    writer: Option<FileWriter<BufWriter<File>>>,
    schema: SchemaRef,
}

impl FormatWriter for IpcWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let batch = RecordBatch::try_new(self.schema.clone(), batch.columns().to_vec())
            .map_err(|e| err(format!("retag contratto IPC: {e}")))?;
        self.writer
            .as_mut()
            .ok_or_else(|| err("writer chiuso"))?
            .write(&batch)
            .map_err(|e| err(format!("write IPC: {e}")))
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        let mut w = self.writer.take().ok_or_else(|| err("writer già chiuso"))?;
        w.finish().map_err(|e| err(format!("finish IPC: {e}")))?;
        let mut inner = w
            .into_inner()
            .map_err(|e| err(format!("into_inner: {e}")))?;
        inner.flush()?;
        drop(inner);
        let (bytes, outcome) = self.staging.publish()?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            fidelity: plenora_io_core::FidelityAssessment::lossless(),
            outcome,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use arrow_array::{BinaryArray, Int64Array};
    use arrow_schema::{DataType, Field, Schema, SchemaRef};
    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;
    use plenora_io_model::contract::{
        CoordinateDimensions, CoordinatePrecision, GeometryEncoding, GeometryType, SpatialSemantics,
    };
    use plenora_io_model::wkb::{
        encode_wkb, to_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
    };

    #[test]
    fn geometry_without_crs_metadata_is_explicitly_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing-crs.arrow");
        let field = Field::new("geometry", DataType::Binary, true).with_metadata(
            [(
                plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
                plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
            )]
            .into_iter()
            .collect(),
        );
        let schema = Schema::new(vec![field]);
        {
            let file = File::create(&path).unwrap();
            let mut writer = FileWriter::try_new(file, &schema).unwrap();
            writer.finish().unwrap();
        }

        let dataset = IpcDriver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
        assert!(matches!(
            &dataset.layers()[0].contract.geometry.as_ref().unwrap().crs,
            CrsResolution::Missing
        ));
    }

    #[test]
    fn unresolved_authority_without_definition_is_preserved() {
        use plenora_io_model::geometry::{
            ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, PLENORA_AXIS_ORDER_KEY,
            PLENORA_CRS_DEFINITION_FORMAT_KEY, PLENORA_CRS_DEFINITION_KEY, PLENORA_CRS_ID_KEY,
            PLENORA_CRS_RESOLUTION_KEY,
        };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unresolved-authority.arrow");
        let field = Field::new("geometry", DataType::Binary, true).with_metadata(
            [
                (
                    ARROW_EXTENSION_NAME_KEY.to_owned(),
                    GEOARROW_WKB_EXTENSION.to_owned(),
                ),
                (
                    PLENORA_CRS_RESOLUTION_KEY.to_owned(),
                    "declared_unresolved".to_owned(),
                ),
                (PLENORA_CRS_ID_KEY.to_owned(), "EPSG:99999".to_owned()),
                (PLENORA_AXIS_ORDER_KEY.to_owned(), "unknown".to_owned()),
            ]
            .into_iter()
            .collect(),
        );
        let schema = Schema::new(vec![field]);
        {
            let file = File::create(&path).unwrap();
            let mut writer = FileWriter::try_new(file, &schema).unwrap();
            writer.finish().unwrap();
        }

        let dataset = IpcDriver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
        let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
        let raw = geometry.crs.raw().unwrap();
        assert_eq!(raw.authority_hint.as_deref(), Some("EPSG:99999"));
        assert_eq!(raw.definition, None);
        assert_eq!(raw.definition_format, None);

        let emitted = with_geometry_contract_metadata(
            &Field::new("geometry", DataType::Binary, true),
            geometry,
        );
        assert!(!emitted.metadata().contains_key(PLENORA_CRS_DEFINITION_KEY));
        assert!(!emitted
            .metadata()
            .contains_key(PLENORA_CRS_DEFINITION_FORMAT_KEY));
    }

    #[test]
    fn multiple_geoarrow_fields_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ambiguous.arrow");
        let geometry_field = |name| {
            Field::new(name, DataType::Binary, true).with_metadata(
                [(
                    plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
                    plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
                )]
                .into_iter()
                .collect(),
            )
        };
        let schema = Schema::new(vec![
            geometry_field("geometry_a"),
            geometry_field("geometry_b"),
        ]);
        {
            let file = File::create(&path).unwrap();
            let mut writer = FileWriter::try_new(file, &schema).unwrap();
            writer.finish().unwrap();
        }

        assert!(matches!(
            IpcDriver.open(Source::Path(path), &ReadOptions::default()),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
        ));
    }

    #[test]
    fn round_trip_ipc_preserves_geometry_metadata() {
        use driver_common_geometry_field as geometry_field;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("t.arrow");
        let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
            12.5, 45.9,
        )))
        .unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field("geometry", "EPSG:4326"),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(Int64Array::from(vec![7i64])),
            ],
        )
        .unwrap();

        let driver = IpcDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema: schema.clone(),
                    geometry: None,
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        let ds = driver
            .open(Source::Path(out), &ReadOptions::default())
            .unwrap();
        // Il CRS e la geometria sopravvivono nei metadati Arrow (pass-through).
        let g = ds.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(g.name, "geometry");
        assert_eq!(g.crs.id(), Some("EPSG:4326"));
        let mut r = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget::default(),
                cancellation: Default::default(),
            })
            .unwrap();
        let rb = r.next_batch().unwrap().unwrap();
        assert_eq!(rb.num_rows(), 1);
        assert!(is_geometry_field(
            &rb.schema().field_with_name("geometry").unwrap().clone()
        ));
        let col = rb
            .column_by_name("geometry")
            .unwrap()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(col.value(0), wkb.as_slice());
        assert!(r.next_batch().unwrap().is_none());

        let mut projected = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: Some(vec![FieldId(1)]),
                projection_mode: ProjectionMode::Required,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget::default(),
                cancellation: Default::default(),
            })
            .unwrap();
        assert_eq!(projected.contract().contract.schema.fields().len(), 1);
        assert_eq!(projected.contract().contract.schema.field(0).name(), "id");
        assert!(projected.contract().contract.geometry.is_none());
        let projected_batch = projected.next_batch().unwrap().unwrap();
        assert_eq!(projected_batch.num_columns(), 1);
        assert_eq!(
            projected_batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .value(0),
            7
        );
    }

    #[test]
    fn batch_target_slices_file_defined_ipc_batches() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("batch-target.arrow");
        let schema: SchemaRef =
            Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Int64Array::from(vec![0, 1, 2, 3, 4]))],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "layer".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let driver = IpcDriver;
        let mut writer = driver
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let dataset = driver
            .open(Source::Path(out), &ReadOptions::default())
            .unwrap();
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget {
                    target_bytes: 16,
                    max_rows: 100,
                },
                cancellation: Default::default(),
            })
            .unwrap();
        let mut sizes = Vec::new();
        while let Some(batch) = reader.next_batch().unwrap() {
            sizes.push(batch.num_rows());
        }
        assert_eq!(sizes, vec![2, 2, 1]);
    }

    #[test]
    fn round_trip_ipc_preserves_ewkb_zm_contract_and_bytes() {
        use driver_common_geometry_field as geometry_field;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("zm.arrow");
        let ewkb = encode_wkb(
            &WkbGeometry {
                value: WkbValue::Point(WkbCoordinate {
                    x: 1.0,
                    y: 2.0,
                    z: Some(3.0),
                    m: Some(4.0),
                }),
                dimensions: CoordinateDimensions::Xyzm,
                srid: Some(4326),
            },
            WkbFlavor::Ewkb,
        )
        .unwrap();
        let schema: SchemaRef =
            Arc::new(Schema::new(vec![geometry_field("geometry", "EPSG:4326")]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![Some(ewkb.as_slice())]))],
        )
        .unwrap();
        let mut geometry = GeometryColumnContract::wkb_passthrough(
            FieldId(0),
            "geometry",
            ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
            true,
        );
        geometry.encoding = GeometryEncoding::Ewkb;
        geometry.dimensions = CoordinateDimensions::Xyzm;
        geometry.spatial_semantics = SpatialSemantics::Geography;
        geometry.srid = Some(4326);
        geometry.precision = CoordinatePrecision::Native;
        geometry.set_exact_geometry_types(vec![GeometryType::Point]);
        geometry.native_metadata.insert(
            "postgis.typmod".to_owned(),
            "geography(PointZM,4326)".to_owned(),
        );
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry),
                },
            }],
        };
        let driver = IpcDriver;
        let mut writer = driver
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let dataset = driver
            .open(Source::Path(out), &ReadOptions::default())
            .unwrap();
        let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(geometry.encoding, GeometryEncoding::Ewkb);
        assert_eq!(geometry.dimensions, CoordinateDimensions::Xyzm);
        assert_eq!(geometry.spatial_semantics, SpatialSemantics::Geography);
        assert_eq!(geometry.srid, Some(4326));
        assert_eq!(geometry.precision, CoordinatePrecision::Native);
        assert_eq!(geometry.geometry_types, vec![GeometryType::Point]);
        assert_eq!(
            geometry
                .native_metadata
                .get("postgis.typmod")
                .map(String::as_str),
            Some("geography(PointZM,4326)")
        );
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget::default(),
                cancellation: Default::default(),
            })
            .unwrap();
        let read = reader.next_batch().unwrap().unwrap();
        let geometry_array = read
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(geometry_array.value(0), ewkb);
    }

    // geometry_field locale (evita la dipendenza driver-common nei test).
    fn driver_common_geometry_field(name: &str, crs: &str) -> Field {
        use std::collections::HashMap;
        let mut md = HashMap::new();
        md.insert(
            plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
            plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
        );
        md.insert(GEO_CRS_KEY.to_owned(), crs.to_owned());
        Field::new(name, DataType::Binary, true).with_metadata(md)
    }
}
