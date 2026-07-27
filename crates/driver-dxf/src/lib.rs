//! driver-dxf — DXF ↔ RecordBatch. **Lettura a fedeltà piena** (motore portato
//! da plenora-dxf-tools): esplosione ricorsiva dei blocchi INSERT con
//! composizione di trasformazioni, OCS→WCS (algoritmo dell'asse arbitrario),
//! tassellazione fine di ARC/CIRCLE/ELLIPSE/SPLINE(NURBS) e archi bulge nelle
//! polilinee, più SOLID/POLYLINE/POINT/TEXT/MTEXT. Geometria in WKB
//! `geoarrow.wkb`. La **scrittura** resta `Approximating` (solo geometria +
//! layer; gli attributi non rappresentabili in DXF sono dichiarati come
//! perdita), da cui la classe di fedeltà del driver. CRS via `assume_crs`.
#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use dxf::entities::{Entity, EntityType, Insert, LwPolyline, ModelPoint};
use dxf::{Block, Drawing, LwPolylineVertex, Point as DxfPoint, Vector};
use geo_types::{Coord, Geometry, LineString, Point, Polygon};

mod geometry;
use geometry::{
    tessellate_arc, tessellate_bulge, tessellate_circle, tessellate_ellipse, tessellate_spline,
    Transform,
};

use driver_common::{geometry_field, json_from_array};
use plenora_core::contract::{
    DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId,
};
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
use plenora_io_core::publish::publish_file_atomic_limited;
use plenora_io_core::request::ReadRequest;
use plenora_io_core::{
    validate_write, with_write_validation, AttributeWriteSupport, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, TypeCoercionPolicy, WritePlan, SCALAR_TYPES,
    UTF8_FIELD_NAMES, WKB_XY_GEOMETRY,
};

const GEOMETRY: &str = "geometry";
/// Segmenti per un giro intero (archi, cerchi, ellissi, bulge).
const ARC_SEGMENTS: usize = 24;
/// Limite di annidamento dei blocchi INSERT (anti-ricorsione patologica).
const MAX_INSERT_DEPTH: usize = 16;
/// Tetto complessivo di entità generate (anti-esplosione da array/annidamenti).
const MAX_ENTITIES: usize = 5_000_000;

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
    write_capabilities: Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::LossReported,
        attributes: AttributeWriteSupport::LossReported,
        geometry: WKB_XY_GEOMETRY,
        crs: CrsWriteSupport::None,
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: false,
    }),
    semantic_version: 1,
    driver_version: 1,
    descriptor_version: 2,
};

pub struct DxfDriver;

impl FormatDriver for DxfDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = source.into_path_checked(&opts.limits)?;
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
        validate_write(self.descriptor(), plan, &opts.limits)?;
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
        Ok(with_write_validation(
            Box::new(DxfWriterState {
                drawing: Drawing::new(),
                path,
                durable: opts.durable,
                loss: LossReport::default(),
                dropped_cols: Vec::new(),
                rows: 0,
                first: true,
                wkb_limits: opts.limits.effective_wkb(),
                max_output_bytes: opts.limits.max_output_bytes,
            }),
            self.descriptor(),
            plan,
            opts.limits,
        ))
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
    wkb_limits: WkbLimits,
    max_output_bytes: u64,
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
        let limits = self.wkb_limits;
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
            self.loss.record(
                &format!("attributo non rappresentato in DXF: {c}"),
                self.rows,
            );
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
        let (bytes, outcome) =
            publish_file_atomic_limited(temp, &self.path, self.durable, self.max_output_bytes)?;
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
                loss.record(
                    "anelli interni Polygon scartati (DXF)",
                    pl.interiors().len() as u64,
                );
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

/// Trasformazione OCS→WCS dall'extrusion direction di un'entità.
fn ocs_of(normal: &Vector) -> Transform {
    Transform::ocs([normal.x, normal.y, normal.z])
}

/// Mappa i punti locali (già tassellati) in coordinate WCS via la trasformazione.
fn mapped(local: &[[f64; 2]], t: &Transform) -> Vec<Coord<f64>> {
    local
        .iter()
        .map(|p| {
            let m = t.apply(*p);
            c(m[0], m[1])
        })
        .collect()
}

/// Walker ricorsivo: converte le entità (esplodendo gli INSERT) in righe
/// colonnari WKB. È il motore di plenora-dxf-tools adattato all'interfaccia del
/// driver (geo_types + LossReport invece di GeoJSON).
struct Walker<'a> {
    blocks: HashMap<String, &'a Block>,
    wkb: Vec<Option<Vec<u8>>>,
    layers: Vec<Option<String>>,
    types: Vec<Option<String>>,
    texts: Vec<Option<String>>,
    loss: LossReport,
    budget: usize,
}

impl<'a> Walker<'a> {
    fn new(drawing: &'a Drawing) -> Self {
        Walker {
            blocks: drawing.blocks().map(|b| (b.name.clone(), b)).collect(),
            wkb: Vec::new(),
            layers: Vec::new(),
            types: Vec::new(),
            texts: Vec::new(),
            loss: LossReport::default(),
            budget: MAX_ENTITIES,
        }
    }

    /// Il layer "0" (o vuoto) eredita quello del contesto (blocco padre).
    fn effective_layer(layer: &str, context: &str) -> String {
        if layer.is_empty() || layer == "0" {
            context.to_owned()
        } else {
            layer.to_owned()
        }
    }

    fn push(
        &mut self,
        geom: Geometry<f64>,
        layer: &str,
        ty: &'static str,
        text: Option<String>,
    ) -> Result<()> {
        self.wkb.push(Some(to_wkb(&geom)?));
        self.layers.push(Some(layer.to_owned()));
        self.types.push(Some(ty.to_owned()));
        self.texts.push(text);
        Ok(())
    }

    fn walk_entity(
        &mut self,
        entity: &Entity,
        transform: Transform,
        context: &str,
        depth: usize,
        visiting: &mut HashSet<String>,
    ) -> Result<()> {
        self.budget = self.budget.saturating_sub(1);
        if self.budget == 0 {
            return Err(err(format!("DXF oltre il limite di {MAX_ENTITIES} entità")));
        }
        let layer = Self::effective_layer(&entity.common.layer, context);
        match &entity.specific {
            EntityType::Line(l) => {
                // LINE è già in WCS: nessun OCS.
                let p1 = transform.apply([l.p1.x, l.p1.y]);
                let p2 = transform.apply([l.p2.x, l.p2.y]);
                self.push(
                    Geometry::LineString(LineString(vec![c(p1[0], p1[1]), c(p2[0], p2[1])])),
                    &layer,
                    "LINE",
                    None,
                )?;
            }
            EntityType::LwPolyline(p) => {
                let t = transform.then(ocs_of(&p.extrusion_direction));
                let verts: Vec<([f64; 2], f64)> = p
                    .vertices
                    .iter()
                    .map(|v| (t.apply([v.x, v.y]), v.bulge))
                    .collect();
                self.emit_polyline(&layer, &verts, p.flags & 1 == 1)?;
            }
            EntityType::Polyline(p) => {
                let t = transform.then(ocs_of(&p.normal));
                let verts: Vec<([f64; 2], f64)> = p
                    .vertices()
                    .map(|v| (t.apply([v.location.x, v.location.y]), v.bulge))
                    .collect();
                self.emit_polyline(&layer, &verts, p.is_closed())?;
            }
            EntityType::Circle(cir) => {
                let t = transform.then(ocs_of(&cir.normal));
                let local =
                    tessellate_circle([cir.center.x, cir.center.y], cir.radius, ARC_SEGMENTS);
                if local.len() < 4 {
                    self.loss.record("CIRCLE degenere", 1);
                } else {
                    self.push(
                        Geometry::Polygon(Polygon::new(LineString(mapped(&local, &t)), vec![])),
                        &layer,
                        "CIRCLE",
                        None,
                    )?;
                }
            }
            EntityType::Arc(a) => {
                let t = transform.then(ocs_of(&a.normal));
                let local = tessellate_arc(
                    [a.center.x, a.center.y],
                    a.radius,
                    a.start_angle,
                    a.end_angle,
                    ARC_SEGMENTS,
                );
                if local.len() < 2 {
                    self.loss.record("ARC degenere", 1);
                } else {
                    self.push(
                        Geometry::LineString(LineString(mapped(&local, &t))),
                        &layer,
                        "ARC",
                        None,
                    )?;
                }
            }
            EntityType::ModelPoint(pt) => {
                let t = transform.then(ocs_of(&pt.extrusion_direction));
                let m = t.apply([pt.location.x, pt.location.y]);
                self.push(
                    Geometry::Point(Point::new(m[0], m[1])),
                    &layer,
                    "POINT",
                    None,
                )?;
            }
            EntityType::Text(txt) => {
                let t = transform.then(ocs_of(&txt.normal));
                let m = t.apply([txt.location.x, txt.location.y]);
                self.emit_text(&layer, m, &txt.value, "TEXT")?;
            }
            EntityType::MText(txt) => {
                // MTEXT usa un insertion point già in WCS.
                let m = transform.apply([txt.insertion_point.x, txt.insertion_point.y]);
                self.emit_text(&layer, m, &txt.text, "MTEXT")?;
            }
            EntityType::Solid(s) => {
                let t = transform.then(ocs_of(&s.extrusion_direction));
                // Ordine di traversata del quadrilatero DXF: 1,2,4,3.
                let corners = [
                    t.apply([s.first_corner.x, s.first_corner.y]),
                    t.apply([s.second_corner.x, s.second_corner.y]),
                    t.apply([s.fourth_corner.x, s.fourth_corner.y]),
                    t.apply([s.third_corner.x, s.third_corner.y]),
                ];
                let mut ring: Vec<Coord<f64>> = corners.iter().map(|p| c(p[0], p[1])).collect();
                ring.push(ring[0]);
                self.push(
                    Geometry::Polygon(Polygon::new(LineString(ring), vec![])),
                    &layer,
                    "SOLID",
                    None,
                )?;
            }
            EntityType::Ellipse(el) => {
                let t = transform.then(ocs_of(&el.normal));
                let local = tessellate_ellipse(
                    [el.center.x, el.center.y],
                    [el.major_axis.x, el.major_axis.y],
                    el.minor_axis_ratio,
                    el.start_parameter,
                    el.end_parameter,
                    ARC_SEGMENTS,
                );
                if local.len() < 2 {
                    self.loss.record("ELLIPSE degenere", 1);
                } else {
                    let full = local.first() == local.last() && local.len() >= 4;
                    let coords = mapped(&local, &t);
                    if full {
                        self.push(
                            Geometry::Polygon(Polygon::new(LineString(coords), vec![])),
                            &layer,
                            "ELLIPSE",
                            None,
                        )?;
                    } else {
                        self.push(
                            Geometry::LineString(LineString(coords)),
                            &layer,
                            "ELLIPSE",
                            None,
                        )?;
                    }
                }
            }
            EntityType::Spline(sp) => {
                // I control point della SPLINE sono in WCS: nessun OCS.
                let controls: Vec<[f64; 2]> =
                    sp.control_points.iter().map(|p| [p.x, p.y]).collect();
                let samples = controls.len().max(2) * 6;
                let local = tessellate_spline(
                    sp.degree_of_curve.max(1) as usize,
                    &sp.knot_values,
                    &controls,
                    &sp.weight_values,
                    samples,
                );
                if local.len() < 2 {
                    self.loss.record("SPLINE degenere", 1);
                } else {
                    let closed = sp.flags & 1 == 1;
                    let mut coords = mapped(&local, &transform);
                    if closed {
                        if coords.first() != coords.last() {
                            coords.push(coords[0]);
                        }
                        if coords.len() >= 4 {
                            self.push(
                                Geometry::Polygon(Polygon::new(LineString(coords), vec![])),
                                &layer,
                                "SPLINE",
                                None,
                            )?;
                        } else {
                            self.loss.record("SPLINE degenere", 1);
                        }
                    } else {
                        self.push(
                            Geometry::LineString(LineString(coords)),
                            &layer,
                            "SPLINE",
                            None,
                        )?;
                    }
                }
            }
            EntityType::Insert(insert) => {
                self.walk_insert(insert, transform, &layer, depth, visiting)?;
            }
            EntityType::Region(_) | EntityType::Body(_) => {
                self.loss.record("REGION/BODY (ACIS) non convertibile", 1);
            }
            EntityType::AttributeDefinition(_)
            | EntityType::Attribute(_)
            | EntityType::Seqend(_)
            | EntityType::Vertex(_) => {
                // Elementi di struttura/template: nessuna geometria autonoma.
            }
            _ => self.loss.record("entità DXF non gestita", 1),
        }
        Ok(())
    }

    fn emit_polyline(
        &mut self,
        layer: &str,
        verts: &[([f64; 2], f64)],
        closed: bool,
    ) -> Result<()> {
        if verts.len() < 2 {
            self.loss.record("polilinea degenere (<2 vertici)", 1);
            return Ok(());
        }
        let mut positions = polyline_positions(verts, closed);
        if closed {
            if positions.first() != positions.last() {
                positions.push(positions[0]);
            }
            if positions.len() < 4 {
                self.loss.record("polilinea chiusa degenere", 1);
                return Ok(());
            }
            self.push(
                Geometry::Polygon(Polygon::new(LineString(positions), vec![])),
                layer,
                "LWPOLYLINE",
                None,
            )?;
        } else {
            self.push(
                Geometry::LineString(LineString(positions)),
                layer,
                "LWPOLYLINE",
                None,
            )?;
        }
        Ok(())
    }

    fn emit_text(
        &mut self,
        layer: &str,
        at: [f64; 2],
        value: &str,
        kind: &'static str,
    ) -> Result<()> {
        let cleaned = value.trim();
        let text = if cleaned.is_empty() {
            None
        } else {
            Some(cleaned.to_owned())
        };
        self.push(Geometry::Point(Point::new(at[0], at[1])), layer, kind, text)
    }

    fn walk_insert(
        &mut self,
        insert: &Insert,
        transform: Transform,
        layer: &str,
        depth: usize,
        visiting: &mut HashSet<String>,
    ) -> Result<()> {
        if depth >= MAX_INSERT_DEPTH {
            return Err(err(format!(
                "annidamento INSERT oltre il limite di {MAX_INSERT_DEPTH}"
            )));
        }
        // L'INSERT ha una propria OCS: inserimento, rotazione e scala vi sono espressi.
        let base = transform.then(ocs_of(&insert.extrusion_direction));
        let at = base.apply([insert.location.x, insert.location.y]);
        // Timbro: se l'INSERT porta attributi, emette un punto all'inserimento col
        // nome del blocco (i valori dei tag non hanno colonna → perdita dichiarata).
        let tags = insert
            .attributes()
            .filter(|a| !a.attribute_tag.trim().is_empty())
            .count() as u64;
        if tags > 0 {
            self.push(
                Geometry::Point(Point::new(at[0], at[1])),
                layer,
                "INSERT",
                Some(insert.name.clone()),
            )?;
            self.loss.record(
                "valori attributi INSERT non rappresentati come colonne",
                tags,
            );
        }

        if !visiting.insert(insert.name.clone()) {
            return Err(err(format!(
                "riferimento ciclico al blocco '{}'",
                insert.name
            )));
        }
        let composed = base.then(Transform::insert(
            [insert.location.x, insert.location.y],
            insert.rotation,
            if insert.x_scale_factor == 0.0 {
                1.0
            } else {
                insert.x_scale_factor
            },
            if insert.y_scale_factor == 0.0 {
                1.0
            } else {
                insert.y_scale_factor
            },
        ));
        if let Some(block) = self.blocks.get(&insert.name).copied() {
            for entity in &block.entities {
                self.walk_entity(entity, composed, layer, depth + 1, visiting)?;
            }
        } else if insert.attributes().next().is_none() {
            self.loss.record("blocco INSERT assente", 1);
        }
        visiting.remove(&insert.name);
        Ok(())
    }
}

/// Punti di una polilinea con eventuali archi (bulge) tassellati fra i vertici.
fn polyline_positions(verts: &[([f64; 2], f64)], closed: bool) -> Vec<Coord<f64>> {
    let mut out: Vec<Coord<f64>> = Vec::with_capacity(verts.len());
    let n = verts.len();
    for i in 0..n {
        let (point, bulge) = verts[i];
        out.push(c(point[0], point[1]));
        let next = if i + 1 < n {
            Some(verts[i + 1].0)
        } else if closed {
            Some(verts[0].0)
        } else {
            None
        };
        if let Some(next) = next {
            for ap in tessellate_bulge(point, next, bulge, ARC_SEGMENTS) {
                out.push(c(ap[0], ap[1]));
            }
        }
    }
    out
}

fn build_batch(drawing: &Drawing, crs: &str) -> Result<(RecordBatch, LossReport, DataContract)> {
    let mut walker = Walker::new(drawing);
    let mut visiting: HashSet<String> = HashSet::new();
    for e in drawing.entities() {
        walker.walk_entity(e, Transform::IDENTITY, "0", 0, &mut visiting)?;
    }

    let fields = vec![
        geometry_field(GEOMETRY, crs),
        Field::new("layer", DataType::Utf8, true),
        Field::new("dxf_type", DataType::Utf8, true),
        Field::new("text", DataType::Utf8, true),
    ];
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(BinaryArray::from(
            walker.wkb.iter().map(|o| o.as_deref()).collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(walker.layers)),
        Arc::new(StringArray::from(walker.types)),
        Arc::new(StringArray::from(walker.texts)),
    ];
    let schema: SchemaRef = Arc::new(Schema::new(fields));
    let batch =
        RecordBatch::try_new(schema.clone(), arrays).map_err(|e| err(format!("batch: {e}")))?;
    let contract = DataContract {
        schema,
        geometry: Some(GeometryColumnContract::wkb_xy(
            FieldId(0),
            GEOMETRY,
            ResolvedCrs {
                id: Some(crs.to_owned()),
                kind: CrsKind::Unknown,
                definition: None,
            },
            true,
        )),
    };
    Ok((batch, walker.loss, contract))
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
                    ..ReadOptions::default()
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

    #[test]
    fn insert_block_is_exploded_and_translated() {
        // Blocco BOX = una LINE (0,0)-(1,0) su layer "0"; un INSERT lo colloca a
        // (10,5) su layer MYLAYER. L'esplosione deve tradurre la LINE a
        // (10,5)-(11,5) ed ereditare il layer dell'INSERT.
        let dxf = "\
0\nSECTION\n2\nBLOCKS\n\
0\nBLOCK\n8\n0\n2\nBOX\n10\n0.0\n20\n0.0\n30\n0.0\n\
0\nLINE\n8\n0\n10\n0.0\n20\n0.0\n30\n0.0\n11\n1.0\n21\n0.0\n31\n0.0\n\
0\nENDBLK\n\
0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nINSERT\n8\nMYLAYER\n2\nBOX\n10\n10.0\n20\n5.0\n30\n0.0\n\
0\nENDSEC\n0\nEOF\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blk.dxf");
        std::fs::write(&path, dxf).unwrap();
        let drawing = Drawing::load_file(&path).unwrap();
        let (batch, _loss, _contract) = build_batch(&drawing, "EPSG:4326").unwrap();

        let geom = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        let layers = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let types = batch
            .column(2)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let limits = WkbLimits::default();
        let mut found = false;
        for i in 0..batch.num_rows() {
            if types.value(i) == "LINE" {
                if let Geometry::LineString(ls) = from_wkb(geom.value(i), &limits).unwrap() {
                    let p = &ls.0;
                    assert!((p[0].x - 10.0).abs() < 1e-6 && (p[0].y - 5.0).abs() < 1e-6);
                    assert!((p[1].x - 11.0).abs() < 1e-6 && (p[1].y - 5.0).abs() < 1e-6);
                    assert_eq!(layers.value(i), "MYLAYER");
                    found = true;
                }
            }
        }
        assert!(found, "la LINE del blocco esploso non è stata trovata");
    }
}
