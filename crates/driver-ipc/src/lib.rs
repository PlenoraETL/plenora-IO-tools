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
use arrow_schema::SchemaRef;

use plenora_core::contract::{
    DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId,
};
use plenora_core::crs::{CrsKind, ResolvedCrs};
use plenora_core::geometry::{
    is_geometry_field, read_geometry_contract_metadata, with_geometry_contract_metadata,
    GEO_CRS_KEY,
};
use plenora_core::{PlenoraError, Result};
use plenora_io_core::descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, ReadMode, ReaderConcurrency, Runtime,
    WriteMode,
};
use plenora_io_core::driver::{
    FormatDriver, FormatWriter, LayerReader, OpenDatasetHandle, Published, ReadOptions, Sink,
    Source, WriteOptions,
};
use plenora_io_core::loss::LossReport;
use plenora_io_core::publish::publish_file_atomic_limited;
use plenora_io_core::request::ReadRequest;
use plenora_io_core::{
    validate_write, with_write_validation, AttributeWriteSupport, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, TypeCoercionPolicy, WritePlan, ALL_ARROW_TYPES,
    UTF8_FIELD_NAMES, WKB_EWKB_PASSTHROUGH_GEOMETRY,
};

fn err(reason: impl Into<String>) -> PlenoraError {
    PlenoraError::Format {
        driver: "ipc",
        reason: reason.into(),
    }
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "ipc",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingSequential,
    write_mode: Some(WriteMode::Streaming),
    multi_layer: false,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::MultipleIndependentReaders,
    crs_handling: CrsHandling::Embedded, // il CRS viaggia nei metadati del campo
    fidelity_class: Fidelity::Lossless,
    runtime: Runtime::PureRust,
    write_capabilities: Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: ALL_ARROW_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_EWKB_PASSTHROUGH_GEOMETRY,
        crs: CrsWriteSupport::Embedded,
        nullability: NullabilitySupport::Preserve,
        multi_layer: false,
    }),
    semantic_version: 1,
    driver_version: 1,
    descriptor_version: 2,
};

pub struct IpcDriver;

impl FormatDriver for IpcDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = source.into_path_checked(&opts.limits)?;
        let reader = FileReader::try_new(File::open(&path)?, None)
            .map_err(|e| err(format!("Arrow IPC non valido: {e}")))?;
        let schema = reader.schema();
        let geometry = schema
            .fields()
            .iter()
            .position(|f| is_geometry_field(f))
            .map(|i| {
                let f = schema.field(i);
                let mut contract = GeometryColumnContract::wkb_passthrough(
                    FieldId(i as u32),
                    f.name(),
                    ResolvedCrs {
                        id: f.metadata().get(GEO_CRS_KEY).cloned(),
                        kind: CrsKind::Unknown,
                        definition: None,
                    },
                    f.is_nullable(),
                );
                read_geometry_contract_metadata(f, &mut contract);
                contract
            });
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
                contract: DataContract { schema, geometry },
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
            return Err(PlenoraError::OutputExists(path.display().to_string()));
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("arrow"))
        {
            return Err(PlenoraError::Unsupported(
                "l'output deve avere estensione .arrow".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraError::Unsupported(
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
        let schema = Arc::new(arrow_schema::Schema::new_with_metadata(
            fields,
            layer.schema.metadata().clone(),
        ));
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let temp = tempfile::NamedTempFile::new_in(&parent)?;
        let writer = FileWriter::try_new(BufWriter::new(temp.reopen()?), &schema)
            .map_err(|e| err(format!("writer IPC: {e}")))?;
        Ok(with_write_validation(
            Box::new(IpcWriter {
                temp: Some(temp),
                writer: Some(writer),
                path,
                durable: opts.durable,
                schema,
                max_output_bytes: opts.limits.max_output_bytes,
            }),
            self.descriptor(),
            plan,
            opts.limits,
        ))
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
    fn open_layer_reader(&self, _request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        let reader = FileReader::try_new(File::open(&self.path)?, None)
            .map_err(|e| err(format!("Arrow IPC non valido: {e}")))?;
        Ok(Box::new(IpcReader {
            reader,
            layer: self.layers[0].clone(),
        }))
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
    temp: Option<tempfile::NamedTempFile>,
    writer: Option<FileWriter<BufWriter<File>>>,
    path: PathBuf,
    durable: bool,
    schema: SchemaRef,
    max_output_bytes: u64,
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
        let temp = self.temp.take().ok_or_else(|| err("temp mancante"))?;
        let (bytes, outcome) =
            publish_file_atomic_limited(temp, &self.path, self.durable, self.max_output_bytes)?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
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
    use plenora_core::contract::{
        CoordinateDimensions, CoordinatePrecision, GeometryEncoding, GeometryType, SpatialSemantics,
    };
    use plenora_core::wkb::{encode_wkb, to_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};
    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;

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
            ResolvedCrs {
                id: Some("EPSG:4326".to_owned()),
                kind: CrsKind::Geographic,
                definition: None,
            },
            true,
        );
        geometry.encoding = GeometryEncoding::Ewkb;
        geometry.dimensions = CoordinateDimensions::Xyzm;
        geometry.spatial_semantics = SpatialSemantics::Geography;
        geometry.srid = Some(4326);
        geometry.precision = CoordinatePrecision::Native;
        geometry.geometry_types = vec![GeometryType::Point];
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
            plenora_core::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
            plenora_core::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
        );
        md.insert(GEO_CRS_KEY.to_owned(), crs.to_owned());
        Field::new(name, DataType::Binary, true).with_metadata(md)
    }
}
