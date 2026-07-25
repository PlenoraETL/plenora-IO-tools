//! driver-kml — KML → RecordBatch (Fase 1, read-only). KML è WGS84 per specifica
//! (`OGC:CRS84`). I Placemark diventano feature: geometria → WKB `geoarrow.wkb`,
//! `name`/`description` come proprietà. KMZ e scrittura: incrementi successivi.
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use kml::types::{Element, Geometry as KmlGeometry, Placemark};
use kml::{Kml, KmlDocument, KmlVersion, KmlWriter};

use driver_common::{geometry_field, json_from_array, OGC_CRS84};
use plenora_core::contract::{DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId};
use plenora_core::crs::ResolvedCrs;
use plenora_core::geometry::is_geometry_field;
use plenora_core::limits::WkbLimits;
use plenora_core::wkb::{from_wkb, to_wkb};
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
use plenora_io_core::publish::publish_file_atomic;
use plenora_io_core::request::ReadRequest;
use plenora_io_core::WritePlan;

const GEOMETRY: &str = "geometry";

fn err(reason: impl Into<String>) -> PlenoraError {
    PlenoraError::Format {
        driver: "kml",
        reason: reason.into(),
    }
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "kml",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::Materializing,
    write_mode: Some(WriteMode::Buffered),
    multi_layer: false,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::SingleActiveReader,
    crs_handling: CrsHandling::FixedWgs84,
    fidelity_class: Fidelity::Conditional,
    runtime: Runtime::PureRust,
    semantic_version: 1,
    driver_version: 1,
    descriptor_version: 1,
};

pub struct KmlDriver;

impl FormatDriver for KmlDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, _opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let Source::Path(path) = source;
        let text = std::fs::read_to_string(&path)?;
        let root: Kml = text
            .parse()
            .map_err(|e| err(format!("KML non valido: {e}")))?;
        let mut placemarks = Vec::new();
        collect(&root, &mut placemarks);
        let (batch, contract) = build_batch(&placemarks)?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(Box::new(KmlDataset {
            layers: vec![LayerContract {
                id: LayerId(0),
                name,
                contract,
            }],
            batch,
        }))
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        let Sink::Path(path) = sink;
        if path.exists() {
            return Err(PlenoraError::OutputExists(path.display().to_string()));
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("kml"))
        {
            return Err(PlenoraError::Unsupported(
                "l'output deve avere estensione .kml".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraError::Unsupported(
                "KML: un solo layer per file".to_owned(),
            ));
        }
        Ok(Box::new(KmlWriterState {
            path,
            durable: opts.durable,
            placemarks: Vec::new(),
        }))
    }
}

struct KmlDataset {
    layers: Vec<LayerContract>,
    batch: RecordBatch,
}

impl OpenDatasetHandle for KmlDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn open_layer_reader(&self, _request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        Ok(Box::new(KmlReader {
            batch: Some(self.batch.clone()),
            layer: self.layers[0].clone(),
        }))
    }
}

struct KmlReader {
    batch: Option<RecordBatch>,
    layer: LayerContract,
}

impl LayerReader for KmlReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }
    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        Ok(self.batch.take())
    }
}

// --- scrittura -------------------------------------------------------------

struct KmlWriterState {
    path: PathBuf,
    durable: bool,
    placemarks: Vec<Placemark>,
}

impl FormatWriter for KmlWriterState {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let schema = batch.schema();
        let geom_idx = schema
            .fields()
            .iter()
            .position(|f| is_geometry_field(f))
            .ok_or_else(|| err("nessuna colonna geometria geoarrow.wkb"))?;
        let geom_col = batch
            .column(geom_idx)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| err("colonna geometria non binaria"))?;
        let limits = WkbLimits::default();
        let name_idx = schema.index_of("name").ok();
        let desc_idx = schema.index_of("description").ok();

        for row in 0..batch.num_rows() {
            let geometry = if geom_col.is_null(row) {
                None
            } else {
                let g = from_wkb(geom_col.value(row), &limits)?;
                Some(KmlGeometry::from(g))
            };
            let name = name_idx.and_then(|i| cell_string(batch.column(i), row));
            let description = desc_idx.and_then(|i| cell_string(batch.column(i), row));

            // Colonne extra (non name/description/geometria) -> ExtendedData.
            let mut data = Vec::new();
            for (i, f) in schema.fields().iter().enumerate() {
                if i == geom_idx || Some(i) == name_idx || Some(i) == desc_idx {
                    continue;
                }
                if let Some(v) = cell_string(batch.column(i), row) {
                    data.push((f.name().clone(), v));
                }
            }
            let children = if data.is_empty() {
                Vec::new()
            } else {
                vec![extended_data(&data)]
            };

            self.placemarks.push(Placemark {
                name,
                description,
                geometry,
                children,
                ..Default::default()
            });
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<Published> {
        // Avvolge i Placemark in <kml xmlns><Document>…</Document></kml> (root
        // KML valido, altrimenti GDAL/parser rifiutano il file).
        let doc = Kml::KmlDocument(KmlDocument {
            version: KmlVersion::V22,
            attrs: HashMap::from([(
                "xmlns".to_owned(),
                "http://www.opengis.net/kml/2.2".to_owned(),
            )]),
            elements: vec![Kml::Document {
                attrs: HashMap::new(),
                elements: self.placemarks.into_iter().map(Kml::Placemark).collect(),
            }],
        });
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let mut temp = tempfile::NamedTempFile::new_in(&parent)?;
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut w = KmlWriter::from_writer(&mut buf);
            w.write(&doc)
                .map_err(|e| err(format!("serializzazione KML: {e}")))?;
        }
        temp.as_file_mut().write_all(&buf)?;
        temp.as_file_mut().flush()?;
        let (bytes, outcome) = publish_file_atomic(temp, &self.path, self.durable)?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            outcome,
        })
    }
}

fn cell_string(array: &ArrayRef, row: usize) -> Option<String> {
    match json_from_array(array, row) {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s),
        other => Some(other.to_string()),
    }
}

fn extended_data(pairs: &[(String, String)]) -> Element {
    Element {
        name: "ExtendedData".to_owned(),
        attrs: HashMap::new(),
        content: None,
        children: pairs
            .iter()
            .map(|(k, v)| Element {
                name: "Data".to_owned(),
                attrs: HashMap::from([("name".to_owned(), k.clone())]),
                content: None,
                children: vec![Element {
                    name: "value".to_owned(),
                    attrs: HashMap::new(),
                    content: Some(v.clone()),
                    children: Vec::new(),
                }],
            })
            .collect(),
    }
}

fn collect(k: &Kml, out: &mut Vec<Placemark>) {
    match k {
        Kml::KmlDocument(d) => {
            for e in &d.elements {
                collect(e, out);
            }
        }
        Kml::Document { elements, .. } => {
            for e in elements {
                collect(e, out);
            }
        }
        Kml::Folder { elements, .. } => {
            for e in elements {
                collect(e, out);
            }
        }
        Kml::Placemark(p) => out.push(p.clone()),
        _ => {}
    }
}

fn build_batch(placemarks: &[Placemark]) -> Result<(RecordBatch, DataContract)> {
    let mut wkb: Vec<Option<Vec<u8>>> = Vec::with_capacity(placemarks.len());
    let mut names: Vec<Option<String>> = Vec::with_capacity(placemarks.len());
    let mut descs: Vec<Option<String>> = Vec::with_capacity(placemarks.len());
    for p in placemarks {
        match &p.geometry {
            None => wkb.push(None),
            Some(g) => match geo_types::Geometry::<f64>::try_from(g.clone()) {
                Ok(geom) => wkb.push(Some(to_wkb(&geom)?)),
                Err(_) => wkb.push(None),
            },
        }
        names.push(p.name.clone());
        descs.push(p.description.clone());
    }

    let fields = vec![
        geometry_field(GEOMETRY, OGC_CRS84),
        Field::new("name", DataType::Utf8, true),
        Field::new("description", DataType::Utf8, true),
    ];
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(BinaryArray::from(
            wkb.iter().map(|o| o.as_deref()).collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(names)),
        Arc::new(StringArray::from(descs)),
    ];
    let schema: SchemaRef = Arc::new(Schema::new(fields));
    let batch =
        RecordBatch::try_new(schema.clone(), arrays).map_err(|e| err(format!("batch: {e}")))?;
    let contract = DataContract {
        schema,
        geometry: Some(GeometryColumnContract {
            field_id: FieldId(0),
            name: GEOMETRY.to_owned(),
            crs: ResolvedCrs::wgs84(),
            nullable: true,
        }),
    };
    Ok((batch, contract))
}

#[cfg(test)]
mod tests {
    use super::*;
    use plenora_io_core::request::{BatchTarget, ProjectionMode};

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
    <kml xmlns="http://www.opengis.net/kml/2.2"><Document>
      <Placemark><name>A</name><description>primo</description>
        <Point><coordinates>12.5,45.9,0</coordinates></Point></Placemark>
      <Placemark><name>B</name>
        <LineString><coordinates>0,0,0 1,1,0</coordinates></LineString></Placemark>
    </Document></kml>"#;

    #[test]
    fn reads_kml_placemarks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("in.kml");
        std::fs::write(&path, SAMPLE).unwrap();
        let driver = KmlDriver;
        let ds = driver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
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
        let batch = r.next_batch().unwrap().unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 3);
    }

    #[test]
    fn write_then_read_round_trip() {
        use plenora_io_core::WriteLayer;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.kml");
        let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(12.5, 45.9))).unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, OGC_CRS84),
            Field::new("name", DataType::Utf8, true),
            Field::new("description", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(StringArray::from(vec!["Roma"])),
                Arc::new(StringArray::from(vec!["capitale"])),
            ],
        )
        .unwrap();

        let driver = KmlDriver;
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

        let ds = driver.open(Source::Path(out), &ReadOptions::default()).unwrap();
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
        let name = rb
            .column_by_name("name")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(name.value(0), "Roma");
    }
}
