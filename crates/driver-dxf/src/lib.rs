//! driver-dxf — DXF → RecordBatch (Fase 1, read-only, **fedeltà ridotta**).
//! Converte le entità base (LINE, LWPOLYLINE, POLYLINE, CIRCLE, ARC, POINT,
//! TEXT/MTEXT) in WKB `geoarrow.wkb`; le altre (INSERT, SPLINE, HATCH, …) sono
//! contate nel `LossReport`. Il motore pieno di plenora-dxf-tools (esplosione
//! INSERT, OCS, tassellazione fine, CRS embedded) è un incremento successivo.
//! Dichiarato `Approximating`. CRS via `assume_crs`.
#![forbid(unsafe_code)]

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use dxf::entities::{Entity, EntityType, LwPolyline, ModelPoint};
use dxf::{Drawing, LwPolylineVertex, Point as DxfPoint};
use geo_types::{Coord, Geometry, LineString, Point, Polygon};

use driver_common::{geometry_field, json_from_array};
use plenora_core::contract::{DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId};
use plenora_core::crs::{CrsKind, ResolvedCrs};
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
const ARC_SEGMENTS: usize = 24;

fn err(reason: impl Into<String>) -> PlenoraError {
    PlenoraError::Format {
        driver: "dxf",
        reason: reason.into(),
    }
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "dxf",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::Materializing,
    write_mode: Some(WriteMode::Buffered),
    multi_layer: false,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::SingleActiveReader,
    crs_handling: CrsHandling::None, // CRS embedded non ancora estratto: via assume_crs
    fidelity_class: Fidelity::Approximating,
    runtime: Runtime::PureRust,
    semantic_version: 1,
    driver_version: 1,
    descriptor_version: 1,
};

pub struct DxfDriver;

impl FormatDriver for DxfDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let Source::Path(path) = source;
        let drawing = Drawing::load_file(&path).map_err(|e| err(format!("apertura DXF: {e}")))?;
        let crs = opts
            .assume_crs
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());
        let (batch, loss, contract) = build_batch(&drawing, &crs)?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(Box::new(DxfDataset {
            layers: vec![LayerContract {
                id: LayerId(0),
                name,
                contract,
            }],
            batch,
            loss,
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
            .is_some_and(|e| e.eq_ignore_ascii_case("dxf"))
        {
            return Err(PlenoraError::Unsupported(
                "l'output deve avere estensione .dxf".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraError::Unsupported(
                "DXF: un solo layer per file".to_owned(),
            ));
        }
        Ok(Box::new(DxfWriterState {
            drawing: Drawing::new(),
            path,
            durable: opts.durable,
            loss: LossReport::default(),
            dropped_cols: Vec::new(),
            rows: 0,
            first: true,
        }))
    }
}

// --- scrittura (fedeltà Approximating: solo geometria + layer) -------------

struct DxfWriterState {
    drawing: Drawing,
    path: PathBuf,
    durable: bool,
    loss: LossReport,
    dropped_cols: Vec<String>,
    rows: u64,
    first: bool,
}

impl FormatWriter for DxfWriterState {
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
        let layer_idx = schema.index_of("layer").ok();
        if self.first {
            for (i, f) in schema.fields().iter().enumerate() {
                if i != geom_idx && Some(i) != layer_idx {
                    self.dropped_cols.push(f.name().clone());
                }
            }
            self.first = false;
        }
        let limits = WkbLimits::default();
        for row in 0..batch.num_rows() {
            self.rows += 1;
            if geom_col.is_null(row) {
                continue;
            }
            let g = from_wkb(geom_col.value(row), &limits)?;
            let layer = layer_idx.and_then(|i| cell_string(batch.column(i), row));
            add_geometry(&mut self.drawing, &g, layer.as_deref(), &mut self.loss);
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        // Gli attributi non rappresentabili in DXF sono dichiarati come perdita.
        for c in &self.dropped_cols {
            self.loss
                .record(&format!("attributo non rappresentato in DXF: {c}"), self.rows);
        }
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let mut temp = tempfile::NamedTempFile::new_in(&parent)?;
        let mut buf: Vec<u8> = Vec::new();
        self.drawing
            .save(&mut buf)
            .map_err(|e| err(format!("serializzazione DXF: {e}")))?;
        temp.as_file_mut().write_all(&buf)?;
        temp.as_file_mut().flush()?;
        let (bytes, outcome) = publish_file_atomic(temp, &self.path, self.durable)?;
        Ok(Published {
            bytes,
            loss: self.loss,
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

fn add_entity(dr: &mut Drawing, specific: EntityType, layer: Option<&str>) {
    let mut e = Entity::new(specific);
    if let Some(l) = layer {
        e.common.layer = l.to_owned();
    }
    dr.add_entity(e);
}

fn lwpolyline(ls: &LineString<f64>, closed: bool) -> EntityType {
    let vertices = ls
        .coords()
        .map(|c| LwPolylineVertex {
            x: c.x,
            y: c.y,
            ..Default::default()
        })
        .collect();
    EntityType::LwPolyline(LwPolyline {
        flags: if closed { 1 } else { 0 },
        vertices,
        ..Default::default()
    })
}

fn point_entity(x: f64, y: f64) -> EntityType {
    EntityType::ModelPoint(ModelPoint {
        location: DxfPoint::new(x, y, 0.0),
        ..Default::default()
    })
}

fn add_geometry(dr: &mut Drawing, g: &Geometry<f64>, layer: Option<&str>, loss: &mut LossReport) {
    match g {
        Geometry::Point(p) => add_entity(dr, point_entity(p.x(), p.y()), layer),
        Geometry::LineString(ls) => add_entity(dr, lwpolyline(ls, false), layer),
        Geometry::Polygon(pl) => {
            add_entity(dr, lwpolyline(pl.exterior(), true), layer);
            if !pl.interiors().is_empty() {
                loss.record("anelli interni Polygon scartati (DXF)", pl.interiors().len() as u64);
            }
        }
        Geometry::MultiPoint(mp) => {
            for p in mp {
                add_entity(dr, point_entity(p.x(), p.y()), layer);
            }
        }
        Geometry::MultiLineString(ml) => {
            for ls in ml {
                add_entity(dr, lwpolyline(ls, false), layer);
            }
        }
        Geometry::MultiPolygon(mp) => {
            for pl in mp {
                add_entity(dr, lwpolyline(pl.exterior(), true), layer);
                if !pl.interiors().is_empty() {
                    loss.record(
                        "anelli interni Polygon scartati (DXF)",
                        pl.interiors().len() as u64,
                    );
                }
            }
        }
        Geometry::GeometryCollection(gc) => {
            for gg in gc {
                add_geometry(dr, gg, layer, loss);
            }
        }
        _ => loss.record("geometria non rappresentabile in DXF (M/Z)", 1),
    }
}

struct DxfDataset {
    layers: Vec<LayerContract>,
    batch: RecordBatch,
    loss: LossReport,
}

impl OpenDatasetHandle for DxfDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn open_layer_reader(&self, _request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        Ok(Box::new(DxfReader {
            batch: Some(self.batch.clone()),
            layer: self.layers[0].clone(),
            loss: self.loss.clone(),
        }))
    }
}

struct DxfReader {
    batch: Option<RecordBatch>,
    layer: LayerContract,
    loss: LossReport,
}

impl LayerReader for DxfReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }
    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        Ok(self.batch.take())
    }
    fn loss_report(&self) -> LossReport {
        self.loss.clone()
    }
}

fn c(x: f64, y: f64) -> Coord<f64> {
    Coord { x, y }
}

fn tessellate_circle(cx: f64, cy: f64, r: f64) -> Polygon<f64> {
    let mut ring = Vec::with_capacity(ARC_SEGMENTS + 1);
    for i in 0..=ARC_SEGMENTS {
        let a = std::f64::consts::TAU * (i as f64) / (ARC_SEGMENTS as f64);
        ring.push(c(cx + r * a.cos(), cy + r * a.sin()));
    }
    Polygon::new(LineString(ring), vec![])
}

fn tessellate_arc(cx: f64, cy: f64, r: f64, start_deg: f64, end_deg: f64) -> LineString<f64> {
    let (s, e) = (start_deg.to_radians(), {
        let mut e = end_deg.to_radians();
        if e < start_deg.to_radians() {
            e += std::f64::consts::TAU;
        }
        e
    });
    let mut pts = Vec::with_capacity(ARC_SEGMENTS + 1);
    for i in 0..=ARC_SEGMENTS {
        let a = s + (e - s) * (i as f64) / (ARC_SEGMENTS as f64);
        pts.push(c(cx + r * a.cos(), cy + r * a.sin()));
    }
    LineString(pts)
}

fn build_batch(drawing: &Drawing, crs: &str) -> Result<(RecordBatch, LossReport, DataContract)> {
    let mut wkb: Vec<Option<Vec<u8>>> = Vec::new();
    let mut layers: Vec<Option<String>> = Vec::new();
    let mut types: Vec<Option<String>> = Vec::new();
    let mut texts: Vec<Option<String>> = Vec::new();
    let mut loss = LossReport::default();

    let push = |geom: Geometry<f64>,
                layer: &str,
                ty: &'static str,
                text: Option<String>,
                wkb: &mut Vec<Option<Vec<u8>>>,
                layers: &mut Vec<Option<String>>,
                types: &mut Vec<Option<String>>,
                texts: &mut Vec<Option<String>>|
     -> Result<()> {
        wkb.push(Some(to_wkb(&geom)?));
        layers.push(Some(layer.to_owned()));
        types.push(Some(ty.to_owned()));
        texts.push(text);
        Ok(())
    };

    for e in drawing.entities() {
        let layer = e.common.layer.as_str();
        match &e.specific {
            EntityType::Line(l) => push(
                Geometry::LineString(LineString(vec![c(l.p1.x, l.p1.y), c(l.p2.x, l.p2.y)])),
                layer,
                "LINE",
                None,
                &mut wkb,
                &mut layers,
                &mut types,
                &mut texts,
            )?,
            EntityType::LwPolyline(p) => {
                let coords: Vec<Coord<f64>> = p.vertices.iter().map(|v| c(v.x, v.y)).collect();
                if coords.len() >= 2 {
                    let closed = p.flags & 1 == 1;
                    let geom = if closed {
                        let mut ring = coords.clone();
                        if ring.first() != ring.last() {
                            ring.push(ring[0]);
                        }
                        Geometry::Polygon(Polygon::new(LineString(ring), vec![]))
                    } else {
                        Geometry::LineString(LineString(coords))
                    };
                    push(
                        geom,
                        layer,
                        "LWPOLYLINE",
                        None,
                        &mut wkb,
                        &mut layers,
                        &mut types,
                        &mut texts,
                    )?;
                }
            }
            EntityType::Circle(cir) => push(
                Geometry::Polygon(tessellate_circle(cir.center.x, cir.center.y, cir.radius)),
                layer,
                "CIRCLE",
                None,
                &mut wkb,
                &mut layers,
                &mut types,
                &mut texts,
            )?,
            EntityType::Arc(a) => push(
                Geometry::LineString(tessellate_arc(
                    a.center.x,
                    a.center.y,
                    a.radius,
                    a.start_angle,
                    a.end_angle,
                )),
                layer,
                "ARC",
                None,
                &mut wkb,
                &mut layers,
                &mut types,
                &mut texts,
            )?,
            EntityType::ModelPoint(pt) => push(
                Geometry::Point(Point::new(pt.location.x, pt.location.y)),
                layer,
                "POINT",
                None,
                &mut wkb,
                &mut layers,
                &mut types,
                &mut texts,
            )?,
            EntityType::Text(t) => push(
                Geometry::Point(Point::new(t.location.x, t.location.y)),
                layer,
                "TEXT",
                Some(t.value.clone()),
                &mut wkb,
                &mut layers,
                &mut types,
                &mut texts,
            )?,
            EntityType::MText(t) => push(
                Geometry::Point(Point::new(t.insertion_point.x, t.insertion_point.y)),
                layer,
                "MTEXT",
                Some(t.text.clone()),
                &mut wkb,
                &mut layers,
                &mut types,
                &mut texts,
            )?,
            other => {
                loss.record(entity_kind(other), 1);
            }
        }
    }

    let fields = vec![
        geometry_field(GEOMETRY, crs),
        Field::new("layer", DataType::Utf8, true),
        Field::new("dxf_type", DataType::Utf8, true),
        Field::new("text", DataType::Utf8, true),
    ];
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(BinaryArray::from(
            wkb.iter().map(|o| o.as_deref()).collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(layers)),
        Arc::new(StringArray::from(types)),
        Arc::new(StringArray::from(texts)),
    ];
    let schema: SchemaRef = Arc::new(Schema::new(fields));
    let batch =
        RecordBatch::try_new(schema.clone(), arrays).map_err(|e| err(format!("batch: {e}")))?;
    let contract = DataContract {
        schema,
        geometry: Some(GeometryColumnContract {
            field_id: FieldId(0),
            name: GEOMETRY.to_owned(),
            crs: ResolvedCrs {
                id: Some(crs.to_owned()),
                kind: CrsKind::Unknown,
                definition: None,
            },
            nullable: true,
        }),
    };
    Ok((batch, loss, contract))
}

fn entity_kind(e: &EntityType) -> &'static str {
    match e {
        EntityType::Insert(_) => "INSERT",
        EntityType::Spline(_) => "SPLINE",
        EntityType::Polyline(_) => "POLYLINE",
        EntityType::Ellipse(_) => "ELLIPSE",
        _ => "OTHER",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;

    #[test]
    fn write_then_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.dxf");
        let wkb1 = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
        let wkb2 = to_wkb(&Geometry::Point(Point::new(3.0, 4.0))).unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("val", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![
                    Some(wkb1.as_slice()),
                    Some(wkb2.as_slice()),
                ])),
                Arc::new(arrow_array::Int64Array::from(vec![1i64, 2])),
            ],
        )
        .unwrap();

        let driver = DxfDriver;
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
        let published = w.finish().unwrap();
        // "val" non è rappresentabile in DXF -> dichiarato come perdita (Approximating).
        assert!(!published.loss.is_empty());

        let ds = driver
            .open(
                Source::Path(out),
                &ReadOptions {
                    assume_crs: Some("EPSG:4326".to_owned()),
                    format_options: Default::default(),
                },
            )
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
        let rb = r.next_batch().unwrap().unwrap();
        assert_eq!(rb.num_rows(), 2);
    }
}
