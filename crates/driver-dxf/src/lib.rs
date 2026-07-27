//! driver-dxf — DXF ↔ RecordBatch. Lettura delle primitive native e delle
//! approssimazioni curve con coordinate WKB XY/XYZ, senza passare da
//! `geo-types`. Il motore esplode ricorsivamente i blocchi INSERT con
//! composizione di trasformazioni, OCS→WCS (algoritmo dell'asse arbitrario),
//! tassellazione fine di ARC/CIRCLE/ELLIPSE/SPLINE(NURBS) e archi bulge nelle
//! polilinee, più SOLID/POLYLINE/POINT/TEXT/MTEXT. La **scrittura** resta
//! `Approximating`: gli attributi, i multipart e gli anelli interni non sono
//! nativi. Il CRS è letto/scritto tramite GEODATA e può essere assunto
//! esplicitamente quando il metadato è assente o non risolvibile.
#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use dxf::entities::{Entity, EntityType, Insert, LwPolyline, ModelPoint, Polyline, Vertex};
use dxf::enums::AcadVersion;
use dxf::objects::{GeoData, Object, ObjectType};
use dxf::{Block, Drawing, LwPolylineVertex, Point as DxfPoint, Vector};

mod geometry;
use geometry::{
    tessellate_arc, tessellate_bulge, tessellate_circle, tessellate_ellipse3, tessellate_spline3,
    Transform3,
};

use driver_common::{geometry_field, json_from_array};
use plenora_core::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId,
};
use plenora_core::crs::{CrsKind, RawCrs, ResolvedCrs};
use plenora_core::geometry::{is_geometry_field, with_geometry_contract_metadata};
use plenora_core::limits::{Limits, WkbLimits};
use plenora_core::wkb::{decode_wkb, encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};
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
    FormatWriteCapabilities, NullabilitySupport, SingleReaderGate, TypeCoercionPolicy, WritePlan,
    SCALAR_TYPES, UTF8_FIELD_NAMES, WKB_XY_XYZ_GEOMETRY,
};

const GEOMETRY: &str = "geometry";
/// Segmenti per un giro intero (archi, cerchi, ellissi, bulge).
const ARC_SEGMENTS: usize = 24;
/// Limite di annidamento dei blocchi INSERT (anti-ricorsione patologica).
const MAX_INSERT_DEPTH: usize = 16;
/// Tetto complessivo di entità generate (anti-esplosione da array/annidamenti).
const MAX_ENTITIES: usize = 5_000_000;
const WGS84_ESRI_WKT: &str = "GEOGCS[\"WGS 84\",DATUM[\"D_WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"Degree\",0.0174532925199433],AUTHORITY[\"EPSG\",\"4326\"]]";

fn err(reason: impl Into<String>) -> PlenoraError {
    PlenoraError::Format {
        driver: "dxf",
        reason: reason.into(),
    }
}

fn crs_kind(id: Option<&str>, definition: Option<&str>) -> CrsKind {
    let id = id.unwrap_or_default();
    let definition = definition.unwrap_or_default().to_ascii_uppercase();
    if id == "OGC:CRS84"
        || id == "EPSG:4326"
        || definition.contains("GEOGCS[")
        || definition.contains("GEODCRS[")
    {
        CrsKind::Geographic
    } else if definition.contains("PROJCS[") || definition.contains("PROJCRS[") {
        CrsKind::Projected
    } else {
        CrsKind::Unknown
    }
}

fn digits_after(text: &str, marker: &str) -> Option<String> {
    let start = text.find(marker)? + marker.len();
    let tail = text[start..].trim_start_matches([' ', '"', '\'', ':']);
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}

fn epsg_from_definition(definition: &str) -> Option<String> {
    let upper = definition.to_ascii_uppercase();
    if let Some(code) = digits_after(&upper, "EPSG:") {
        return Some(format!("EPSG:{code}"));
    }
    for marker in ["AUTHORITY[\"EPSG\",", "ID[\"EPSG\","] {
        if let Some(code) = digits_after(&upper, marker) {
            return Some(format!("EPSG:{code}"));
        }
    }
    let mut remaining = upper.as_str();
    while let Some(start) = remaining.find("<ALIAS") {
        remaining = &remaining[start..];
        let end = remaining
            .find("</ALIAS>")
            .map(|index| index + "</ALIAS>".len())
            .unwrap_or(remaining.len());
        let alias = &remaining[..end];
        if alias.contains("EPSG") {
            if let Some(code) = digits_after(alias, "ID=") {
                return Some(format!("EPSG:{code}"));
            }
        }
        remaining = &remaining[end..];
    }
    None
}

fn embedded_crs_definition(drawing: &Drawing) -> Option<String> {
    drawing.objects().find_map(|object| match &object.specific {
        ObjectType::GeoData(geodata) => {
            let definition = geodata.coordinate_system_definition.trim();
            (!definition.is_empty()).then(|| definition.to_owned())
        }
        _ => None,
    })
}

fn resolve_dxf_crs(drawing: &Drawing, options: &ReadOptions) -> Result<ResolvedCrs> {
    match embedded_crs_definition(drawing) {
        Some(definition) => {
            let embedded_id = epsg_from_definition(&definition).or_else(|| {
                let trimmed = definition.trim();
                (trimmed.eq_ignore_ascii_case("OGC:CRS84")).then(|| "OGC:CRS84".to_owned())
            });
            let id = embedded_id.or_else(|| options.assume_crs.clone());
            let Some(id) = id else {
                let raw = RawCrs {
                    definition: definition.clone(),
                    authority_hint: None,
                };
                return Err(PlenoraError::Crs(format!(
                    "DXF con GEODATA dichiarato ma non risolvibile: {raw:?}; fornire --assume-crs"
                )));
            };
            Ok(ResolvedCrs {
                kind: crs_kind(Some(&id), Some(&definition)),
                id: Some(id),
                definition: Some(definition),
            })
        }
        None => {
            let id = options.assume_crs.clone().ok_or_else(|| {
                PlenoraError::Crs("DXF senza GEODATA risolvibile: fornire --assume-crs".to_owned())
            })?;
            Ok(ResolvedCrs {
                kind: crs_kind(Some(&id), None),
                id: Some(id),
                definition: None,
            })
        }
    }
}

fn definition_for_write(geometry: &GeometryColumnContract) -> Result<String> {
    let resolved = geometry
        .resolved_crs()
        .ok_or_else(|| PlenoraError::Crs("DXF richiede un CRS risolto nel contratto".to_owned()))?;
    if let Some(definition) = &resolved.definition {
        if !definition.trim().is_empty() {
            return Ok(definition.clone());
        }
    }
    match resolved.id.as_deref() {
        Some("EPSG:4326") | Some("OGC:CRS84") => Ok(WGS84_ESRI_WKT.to_owned()),
        Some(id) => Err(PlenoraError::Crs(format!(
            "DXF richiede la definizione WKT/XML del CRS {id}, non il solo authority id"
        ))),
        None => Err(PlenoraError::Crs(
            "DXF richiede una definizione WKT/XML del CRS".to_owned(),
        )),
    }
}

fn embed_dxf_crs(drawing: &mut Drawing, geometry: &GeometryColumnContract) -> Result<()> {
    let definition = definition_for_write(geometry)?;
    drawing.header.version = AcadVersion::R2010;
    let geodata = GeoData {
        coordinate_system_definition: definition,
        ..Default::default()
    };
    drawing.add_object(Object::new(ObjectType::GeoData(geodata)));
    Ok(())
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "dxf",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::Materializing,
    write_mode: Some(WriteMode::Buffered),
    multi_layer: false,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::SingleActiveReader,
    crs_handling: CrsHandling::Embedded,
    fidelity_class: Fidelity::Approximating,
    runtime: Runtime::PureRust,
    write_capabilities: Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::LossReported,
        attributes: AttributeWriteSupport::LossReported,
        geometry: WKB_XY_XYZ_GEOMETRY,
        crs: CrsWriteSupport::Embedded,
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: false,
    }),
    semantic_version: 1,
    driver_version: 2,
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
        let crs = resolve_dxf_crs(&drawing, opts)?;
        let (batch, loss, contract) = build_batch(&drawing, crs, &opts.limits)?;
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
            reader_gate: SingleReaderGate::new(DESCRIPTOR.id),
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
        let mut drawing = Drawing::new();
        let geometry =
            plan.layers[0].contract.geometry.as_ref().ok_or_else(|| {
                err("DXF richiede un contratto geometrico esplicito con CRS risolto")
            })?;
        embed_dxf_crs(&mut drawing, geometry)?;
        Ok(with_write_validation(
            Box::new(DxfWriterState {
                drawing,
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
            let g = decode_wkb(geom_col.value(row), &limits)?;
            let layer = layer_idx.and_then(|i| cell_string(batch.column(i), row));
            add_geometry(&mut self.drawing, &g, layer.as_deref(), &mut self.loss)?;
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
            fidelity: plenora_io_core::FidelityAssessment::lossless(),
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

fn lwpolyline(coordinates: &[WkbCoordinate], closed: bool) -> EntityType {
    let coordinates = ring_without_duplicate_end(coordinates, closed);
    let vertices = coordinates
        .iter()
        .map(|coordinate| LwPolylineVertex {
            x: coordinate.x,
            y: coordinate.y,
            ..Default::default()
        })
        .collect();
    EntityType::LwPolyline(LwPolyline {
        flags: if closed { 1 } else { 0 },
        vertices,
        ..Default::default()
    })
}

fn point_entity(coordinate: &WkbCoordinate) -> Result<EntityType> {
    validate_dxf_coordinate(coordinate)?;
    Ok(EntityType::ModelPoint(ModelPoint {
        location: DxfPoint::new(coordinate.x, coordinate.y, coordinate.z.unwrap_or(0.0)),
        ..Default::default()
    }))
}

fn ring_without_duplicate_end(coordinates: &[WkbCoordinate], closed: bool) -> &[WkbCoordinate] {
    if closed && coordinates.len() > 1 && coordinates.first() == coordinates.last() {
        &coordinates[..coordinates.len() - 1]
    } else {
        coordinates
    }
}

fn validate_dxf_coordinate(coordinate: &WkbCoordinate) -> Result<()> {
    if coordinate.m.is_some() {
        return Err(err("DXF non rappresenta ordinate M"));
    }
    if !coordinate.x.is_finite()
        || !coordinate.y.is_finite()
        || coordinate.z.is_some_and(|value| !value.is_finite())
    {
        return Err(err("DXF non rappresenta coordinate non finite"));
    }
    Ok(())
}

fn add_polyline(
    drawing: &mut Drawing,
    coordinates: &[WkbCoordinate],
    closed: bool,
    dimensions: CoordinateDimensions,
    layer: Option<&str>,
) -> Result<()> {
    let coordinates = ring_without_duplicate_end(coordinates, closed);
    for coordinate in coordinates {
        validate_dxf_coordinate(coordinate)?;
    }
    if coordinates.len() < 2 {
        return Err(err("polilinea con meno di due coordinate"));
    }
    match dimensions {
        CoordinateDimensions::Xy => add_entity(drawing, lwpolyline(coordinates, closed), layer),
        CoordinateDimensions::Xyz => {
            let mut polyline = Polyline::default();
            polyline.set_is_closed(closed);
            polyline.set_is_3d_polyline(true);
            for coordinate in coordinates {
                let mut vertex = Vertex::new(DxfPoint::new(
                    coordinate.x,
                    coordinate.y,
                    coordinate.z.expect("XYZ validato"),
                ));
                vertex.set_is_3d_polyline_vertex(true);
                polyline.add_vertex(drawing, vertex);
            }
            add_entity(drawing, EntityType::Polyline(polyline), layer);
        }
        CoordinateDimensions::Xym | CoordinateDimensions::Xyzm | CoordinateDimensions::Unknown => {
            return Err(err(format!(
                "dimensionalità {:?} non rappresentabile in DXF",
                dimensions
            )))
        }
    }
    Ok(())
}

fn add_geometry(
    drawing: &mut Drawing,
    geometry: &WkbGeometry,
    layer: Option<&str>,
    loss: &mut LossReport,
) -> Result<()> {
    if geometry.srid.is_some() {
        return Err(err(
            "SRID EWKB embedded non rappresentabile; usare il CRS GEODATA",
        ));
    }
    match &geometry.value {
        WkbValue::Point(point) => add_entity(drawing, point_entity(point)?, layer),
        WkbValue::LineString(line) => {
            add_polyline(drawing, line, false, geometry.dimensions, layer)?
        }
        WkbValue::Polygon(rings) => {
            let exterior = rings
                .first()
                .ok_or_else(|| err("Polygon senza anello esterno"))?;
            add_polyline(drawing, exterior, true, geometry.dimensions, layer)?;
            if rings.len() > 1 {
                loss.record(
                    "anelli interni Polygon scartati (DXF)",
                    (rings.len() - 1) as u64,
                );
            }
        }
        WkbValue::MultiPoint(points) => {
            loss.record("MultiPoint esploso in entità DXF", points.len() as u64);
            for point in points {
                add_geometry(drawing, point, layer, loss)?;
            }
        }
        WkbValue::MultiLineString(lines) => {
            loss.record("MultiLineString esplosa in entità DXF", lines.len() as u64);
            for line in lines {
                add_geometry(drawing, line, layer, loss)?;
            }
        }
        WkbValue::MultiPolygon(polygons) => {
            loss.record("MultiPolygon esplosa in entità DXF", polygons.len() as u64);
            for polygon in polygons {
                add_geometry(drawing, polygon, layer, loss)?;
            }
        }
        WkbValue::GeometryCollection(collection) => {
            loss.record(
                "GeometryCollection esplosa in entità DXF",
                collection.len() as u64,
            );
            for child in collection {
                add_geometry(drawing, child, layer, loss)?;
            }
        }
    }
    Ok(())
}

struct DxfDataset {
    layers: Vec<LayerContract>,
    batch: RecordBatch,
    loss: LossReport,
    reader_gate: SingleReaderGate,
}

impl OpenDatasetHandle for DxfDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(DESCRIPTOR.id, DESCRIPTOR.fidelity_class)
            .with_loss_report(&self.loss)
    }
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        self.reader_gate.open(request.layer, || {
            Ok(Box::new(DxfReader {
                batch: Some(self.batch.clone()),
                layer: self.layers[0].clone(),
                loss: self.loss.clone(),
            }))
        })
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

fn coordinate(point: [f64; 3]) -> WkbCoordinate {
    WkbCoordinate {
        x: point[0],
        y: point[1],
        z: Some(point[2]),
        m: None,
    }
}

/// Trasformazione OCS→WCS dall'extrusion direction di un'entità.
fn ocs_of(normal: &Vector) -> Transform3 {
    Transform3::ocs([normal.x, normal.y, normal.z])
}

/// Mappa i punti locali (già tassellati) in coordinate WCS via la trasformazione.
fn mapped(local: &[[f64; 2]], elevation: f64, transform: &Transform3) -> Vec<WkbCoordinate> {
    local
        .iter()
        .map(|point| coordinate(transform.apply([point[0], point[1], elevation])))
        .collect()
}

/// Walker ricorsivo: converte le entità (esplodendo gli INSERT) in righe
/// colonnari WKB. È il motore di plenora-dxf-tools adattato all'interfaccia del
/// driver (AST WKB lossless + LossReport invece di GeoJSON).
struct Walker<'a> {
    blocks: HashMap<String, &'a Block>,
    geometries: Vec<Option<WkbGeometry>>,
    layers: Vec<Option<String>>,
    types: Vec<Option<String>>,
    texts: Vec<Option<String>>,
    loss: LossReport,
    budget: usize,
    max_rows: usize,
    remaining_vertices: usize,
}

impl<'a> Walker<'a> {
    fn new(drawing: &'a Drawing, limits: &Limits) -> Self {
        Walker {
            blocks: drawing.blocks().map(|b| (b.name.clone(), b)).collect(),
            geometries: Vec::new(),
            layers: Vec::new(),
            types: Vec::new(),
            texts: Vec::new(),
            loss: LossReport::default(),
            budget: MAX_ENTITIES,
            max_rows: limits.max_rows,
            remaining_vertices: limits.max_vertices,
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
        value: WkbValue,
        layer: &str,
        ty: &'static str,
        text: Option<String>,
    ) -> Result<()> {
        if self.geometries.len() >= self.max_rows {
            return Err(PlenoraError::LimitExceeded(format!(
                "righe DXF oltre il limite di {}",
                self.max_rows
            )));
        }
        let vertices = value_coordinate_count(&value);
        if vertices > self.remaining_vertices {
            return Err(PlenoraError::LimitExceeded(format!(
                "vertici DXF oltre il limite di {}",
                self.remaining_vertices
            )));
        }
        self.remaining_vertices -= vertices;
        self.geometries.push(Some(WkbGeometry {
            value,
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        }));
        self.layers.push(Some(layer.to_owned()));
        self.types.push(Some(ty.to_owned()));
        self.texts.push(text);
        Ok(())
    }

    fn walk_entity(
        &mut self,
        entity: &Entity,
        transform: Transform3,
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
                let p1 = transform.apply([l.p1.x, l.p1.y, l.p1.z]);
                let p2 = transform.apply([l.p2.x, l.p2.y, l.p2.z]);
                self.push(
                    WkbValue::LineString(vec![coordinate(p1), coordinate(p2)]),
                    &layer,
                    "LINE",
                    None,
                )?;
            }
            EntityType::LwPolyline(p) => {
                let object_to_world = transform.then(ocs_of(&p.extrusion_direction));
                let vertices: Vec<([f64; 3], f64)> = p
                    .vertices
                    .iter()
                    .map(|vertex| ([vertex.x, vertex.y, entity.common.elevation], vertex.bulge))
                    .collect();
                self.emit_polyline(
                    &layer,
                    &vertices,
                    p.flags & 1 == 1,
                    object_to_world,
                    "LWPOLYLINE",
                )?;
            }
            EntityType::Polyline(p) => {
                let object_to_world = transform.then(ocs_of(&p.normal));
                let vertices: Vec<([f64; 3], f64)> = p
                    .vertices()
                    .map(|vertex| {
                        (
                            [vertex.location.x, vertex.location.y, vertex.location.z],
                            vertex.bulge,
                        )
                    })
                    .collect();
                self.emit_polyline(
                    &layer,
                    &vertices,
                    p.is_closed(),
                    object_to_world,
                    "POLYLINE",
                )?;
            }
            EntityType::Circle(cir) => {
                let object_to_world = transform.then(ocs_of(&cir.normal));
                let local =
                    tessellate_circle([cir.center.x, cir.center.y], cir.radius, ARC_SEGMENTS);
                if local.len() < 4 {
                    self.loss.record("CIRCLE degenere", 1);
                } else {
                    self.loss.record("CIRCLE tassellata", 1);
                    self.push(
                        WkbValue::Polygon(vec![mapped(&local, cir.center.z, &object_to_world)]),
                        &layer,
                        "CIRCLE",
                        None,
                    )?;
                }
            }
            EntityType::Arc(a) => {
                let object_to_world = transform.then(ocs_of(&a.normal));
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
                    self.loss.record("ARC tassellato", 1);
                    self.push(
                        WkbValue::LineString(mapped(&local, a.center.z, &object_to_world)),
                        &layer,
                        "ARC",
                        None,
                    )?;
                }
            }
            EntityType::ModelPoint(pt) => {
                let object_to_world = transform.then(ocs_of(&pt.extrusion_direction));
                let mapped = object_to_world.apply([pt.location.x, pt.location.y, pt.location.z]);
                self.push(WkbValue::Point(coordinate(mapped)), &layer, "POINT", None)?;
            }
            EntityType::Text(txt) => {
                let object_to_world = transform.then(ocs_of(&txt.normal));
                let mapped =
                    object_to_world.apply([txt.location.x, txt.location.y, txt.location.z]);
                self.emit_text(&layer, mapped, &txt.value, "TEXT")?;
                self.loss.record("TEXT rappresentato come punto", 1);
            }
            EntityType::MText(txt) => {
                // MTEXT usa un insertion point già in WCS.
                let mapped = transform.apply([
                    txt.insertion_point.x,
                    txt.insertion_point.y,
                    txt.insertion_point.z,
                ]);
                self.emit_text(&layer, mapped, &txt.text, "MTEXT")?;
                self.loss.record("MTEXT rappresentato come punto", 1);
            }
            EntityType::Solid(s) => {
                let object_to_world = transform.then(ocs_of(&s.extrusion_direction));
                // Ordine di traversata del quadrilatero DXF: 1,2,4,3.
                let corners = [
                    object_to_world.apply([s.first_corner.x, s.first_corner.y, s.first_corner.z]),
                    object_to_world.apply([
                        s.second_corner.x,
                        s.second_corner.y,
                        s.second_corner.z,
                    ]),
                    object_to_world.apply([
                        s.fourth_corner.x,
                        s.fourth_corner.y,
                        s.fourth_corner.z,
                    ]),
                    object_to_world.apply([s.third_corner.x, s.third_corner.y, s.third_corner.z]),
                ];
                let mut ring: Vec<WkbCoordinate> =
                    corners.iter().copied().map(coordinate).collect();
                ring.push(ring[0]);
                self.push(WkbValue::Polygon(vec![ring]), &layer, "SOLID", None)?;
                self.loss.record("SOLID rappresentato come Polygon", 1);
            }
            EntityType::Ellipse(el) => {
                let local = tessellate_ellipse3(
                    [el.center.x, el.center.y, el.center.z],
                    [el.major_axis.x, el.major_axis.y, el.major_axis.z],
                    [el.normal.x, el.normal.y, el.normal.z],
                    el.minor_axis_ratio,
                    el.start_parameter,
                    el.end_parameter,
                    ARC_SEGMENTS,
                );
                if local.len() < 2 {
                    self.loss.record("ELLIPSE degenere", 1);
                } else {
                    let full = local.first() == local.last() && local.len() >= 4;
                    let coordinates: Vec<WkbCoordinate> = local
                        .iter()
                        .map(|point| coordinate(transform.apply(*point)))
                        .collect();
                    self.loss.record("ELLIPSE tassellata", 1);
                    if full {
                        self.push(
                            WkbValue::Polygon(vec![coordinates]),
                            &layer,
                            "ELLIPSE",
                            None,
                        )?;
                    } else {
                        self.push(WkbValue::LineString(coordinates), &layer, "ELLIPSE", None)?;
                    }
                }
            }
            EntityType::Spline(sp) => {
                // I control point della SPLINE sono in WCS: nessun OCS.
                let controls: Vec<[f64; 3]> = sp
                    .control_points
                    .iter()
                    .map(|point| [point.x, point.y, point.z])
                    .collect();
                let samples = controls.len().max(2) * 6;
                let local = tessellate_spline3(
                    sp.degree_of_curve.max(1) as usize,
                    &sp.knot_values,
                    &controls,
                    &sp.weight_values,
                    samples,
                );
                if local.len() < 2 {
                    self.loss.record("SPLINE degenere", 1);
                } else {
                    self.loss.record("SPLINE tassellata", 1);
                    let closed = sp.flags & 1 == 1;
                    let mut coordinates: Vec<WkbCoordinate> = local
                        .iter()
                        .map(|point| coordinate(transform.apply(*point)))
                        .collect();
                    if closed {
                        if coordinates.first() != coordinates.last() {
                            coordinates.push(coordinates[0]);
                        }
                        if coordinates.len() >= 4 {
                            self.push(
                                WkbValue::Polygon(vec![coordinates]),
                                &layer,
                                "SPLINE",
                                None,
                            )?;
                        } else {
                            self.loss.record("SPLINE degenere", 1);
                        }
                    } else {
                        self.push(WkbValue::LineString(coordinates), &layer, "SPLINE", None)?;
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
        vertices: &[([f64; 3], f64)],
        closed: bool,
        transform: Transform3,
        kind: &'static str,
    ) -> Result<()> {
        if vertices.len() < 2 {
            self.loss.record("polilinea degenere (<2 vertici)", 1);
            return Ok(());
        }
        if vertices.iter().enumerate().any(|(index, (point, bulge))| {
            let next = if index + 1 < vertices.len() {
                Some(vertices[index + 1].0)
            } else if closed {
                Some(vertices[0].0)
            } else {
                None
            };
            *bulge != 0.0 && next.is_some_and(|next| next[2] != point[2])
        }) {
            self.loss.record(
                "bulge DXF con quote diverse approssimato sul piano iniziale",
                1,
            );
        }
        let bulges = vertices
            .iter()
            .filter(|(_, bulge)| bulge.is_finite() && *bulge != 0.0)
            .count() as u64;
        if bulges > 0 {
            self.loss.record("archi bulge tassellati", bulges);
        }
        let mut positions = polyline_positions(vertices, closed, &transform);
        if closed {
            if positions.first() != positions.last() {
                positions.push(positions[0]);
            }
            if positions.len() < 4 {
                self.loss.record("polilinea chiusa degenere", 1);
                return Ok(());
            }
            self.push(WkbValue::Polygon(vec![positions]), layer, kind, None)?;
        } else {
            self.push(WkbValue::LineString(positions), layer, kind, None)?;
        }
        Ok(())
    }

    fn emit_text(
        &mut self,
        layer: &str,
        at: [f64; 3],
        value: &str,
        kind: &'static str,
    ) -> Result<()> {
        let cleaned = value.trim();
        let text = if cleaned.is_empty() {
            None
        } else {
            Some(cleaned.to_owned())
        };
        self.push(WkbValue::Point(coordinate(at)), layer, kind, text)
    }

    fn walk_insert(
        &mut self,
        insert: &Insert,
        transform: Transform3,
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
        let at = base.apply([insert.location.x, insert.location.y, insert.location.z]);
        // Timbro: se l'INSERT porta attributi, emette un punto all'inserimento col
        // nome del blocco (i valori dei tag non hanno colonna → perdita dichiarata).
        let tags = insert
            .attributes()
            .filter(|a| !a.attribute_tag.trim().is_empty())
            .count() as u64;
        if tags > 0 {
            self.push(
                WkbValue::Point(coordinate(at)),
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
        let composed = base.then(Transform3::insert(
            [insert.location.x, insert.location.y, insert.location.z],
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
            if insert.z_scale_factor == 0.0 {
                1.0
            } else {
                insert.z_scale_factor
            },
        ));
        if let Some(block) = self.blocks.get(&insert.name).copied() {
            self.loss.record("blocco INSERT esploso", 1);
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

fn value_coordinate_count(value: &WkbValue) -> usize {
    match value {
        WkbValue::Point(_) => 1,
        WkbValue::LineString(coordinates) => coordinates.len(),
        WkbValue::Polygon(rings) => rings.iter().map(Vec::len).sum(),
        WkbValue::MultiPoint(children)
        | WkbValue::MultiLineString(children)
        | WkbValue::MultiPolygon(children)
        | WkbValue::GeometryCollection(children) => children
            .iter()
            .map(|geometry| value_coordinate_count(&geometry.value))
            .sum(),
    }
}

/// Punti di una polilinea con eventuali archi (bulge) tassellati fra i vertici.
fn polyline_positions(
    vertices: &[([f64; 3], f64)],
    closed: bool,
    transform: &Transform3,
) -> Vec<WkbCoordinate> {
    let mut output = Vec::with_capacity(vertices.len());
    let n = vertices.len();
    for i in 0..n {
        let (point, bulge) = vertices[i];
        output.push(coordinate(transform.apply(point)));
        let next = if i + 1 < n {
            Some(vertices[i + 1].0)
        } else if closed {
            Some(vertices[0].0)
        } else {
            None
        };
        if let Some(next) = next {
            for arc_point in tessellate_bulge(
                [point[0], point[1]],
                [next[0], next[1]],
                bulge,
                ARC_SEGMENTS,
            ) {
                output.push(coordinate(transform.apply([
                    arc_point[0],
                    arc_point[1],
                    point[2],
                ])));
            }
        }
    }
    output
}

fn coordinates_have_nonzero_z(coordinates: &[WkbCoordinate]) -> bool {
    coordinates
        .iter()
        .any(|coordinate| coordinate.z.is_some_and(|z| z != 0.0))
}

fn geometry_has_nonzero_z(geometry: &WkbGeometry) -> bool {
    match &geometry.value {
        WkbValue::Point(coordinate) => coordinate.z.is_some_and(|z| z != 0.0),
        WkbValue::LineString(coordinates) => coordinates_have_nonzero_z(coordinates),
        WkbValue::Polygon(rings) => rings.iter().any(|ring| coordinates_have_nonzero_z(ring)),
        WkbValue::MultiPoint(children)
        | WkbValue::MultiLineString(children)
        | WkbValue::MultiPolygon(children)
        | WkbValue::GeometryCollection(children) => children.iter().any(geometry_has_nonzero_z),
    }
}

fn set_geometry_dimensions(geometry: &mut WkbGeometry, dimensions: CoordinateDimensions) {
    let set_coordinates = |coordinates: &mut [WkbCoordinate]| {
        if dimensions == CoordinateDimensions::Xy {
            for coordinate in coordinates {
                coordinate.z = None;
            }
        }
    };
    match &mut geometry.value {
        WkbValue::Point(coordinate) => {
            if dimensions == CoordinateDimensions::Xy {
                coordinate.z = None;
            }
        }
        WkbValue::LineString(coordinates) => set_coordinates(coordinates),
        WkbValue::Polygon(rings) => {
            for ring in rings {
                set_coordinates(ring);
            }
        }
        WkbValue::MultiPoint(children)
        | WkbValue::MultiLineString(children)
        | WkbValue::MultiPolygon(children)
        | WkbValue::GeometryCollection(children) => {
            for child in children {
                set_geometry_dimensions(child, dimensions);
            }
        }
    }
    geometry.dimensions = dimensions;
}

fn build_batch(
    drawing: &Drawing,
    crs: ResolvedCrs,
    limits: &Limits,
) -> Result<(RecordBatch, LossReport, DataContract)> {
    const DXF_OUTPUT_COLUMNS: usize = 4;
    if limits.max_columns < DXF_OUTPUT_COLUMNS {
        return Err(PlenoraError::LimitExceeded(format!(
            "DXF produce {DXF_OUTPUT_COLUMNS} colonne, oltre il limite di {}",
            limits.max_columns
        )));
    }
    let mut walker = Walker::new(drawing, limits);
    let mut visiting: HashSet<String> = HashSet::new();
    for e in drawing.entities() {
        walker.walk_entity(e, Transform3::IDENTITY, "0", 0, &mut visiting)?;
    }

    let dimensions = if walker
        .geometries
        .iter()
        .flatten()
        .any(geometry_has_nonzero_z)
    {
        CoordinateDimensions::Xyz
    } else {
        CoordinateDimensions::Xy
    };
    let mut geometry_types = BTreeSet::new();
    let mut encoded = Vec::with_capacity(walker.geometries.len());
    for geometry in &mut walker.geometries {
        let Some(geometry) = geometry else {
            encoded.push(None);
            continue;
        };
        set_geometry_dimensions(geometry, dimensions);
        geometry_types.insert(geometry.geometry_type());
        encoded.push(Some(encode_wkb(geometry, WkbFlavor::Iso)?));
    }

    let mut geometry_contract =
        GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, crs.clone(), true);
    geometry_contract.dimensions = dimensions;
    geometry_contract.geometry_types = geometry_types.into_iter().collect();
    geometry_contract
        .native_metadata
        .insert("dxf.geometry_model".to_owned(), "wcs".to_owned());
    geometry_contract.native_metadata.insert(
        "dxf.z_inference".to_owned(),
        "xyz_if_any_nonzero_z_else_xy".to_owned(),
    );
    let crs_label = crs.id.as_deref().unwrap_or("DXF:GEODATA");
    let fields = vec![
        with_geometry_contract_metadata(&geometry_field(GEOMETRY, crs_label), &geometry_contract),
        Field::new("layer", DataType::Utf8, true),
        Field::new("dxf_type", DataType::Utf8, true),
        Field::new("text", DataType::Utf8, true),
    ];
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(BinaryArray::from(
            encoded
                .iter()
                .map(|value| value.as_deref())
                .collect::<Vec<_>>(),
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
        geometry: Some(geometry_contract),
    };
    Ok((batch, walker.loss, contract))
}

/// Entry point non stabile per libFuzzer: esercita il parser DXF e l'intero
/// walker geometrico senza I/O su filesystem. Il CRS sintetico serve solo a
/// completare il contratto dopo il parsing.
#[doc(hidden)]
pub fn __fuzz_read_dxf(bytes: &[u8]) -> Result<usize> {
    use std::io::Cursor;

    const MAX_FUZZ_INPUT_BYTES: usize = 1_048_576;
    if bytes.len() > MAX_FUZZ_INPUT_BYTES {
        return Err(err(format!(
            "input fuzz DXF oltre {MAX_FUZZ_INPUT_BYTES} byte"
        )));
    }
    let drawing = Drawing::load(&mut Cursor::new(bytes))
        .map_err(|error| err(format!("DXF invalido: {error}")))?;
    let crs = ResolvedCrs {
        id: Some("EPSG:4326".to_owned()),
        kind: CrsKind::Geographic,
        definition: None,
    };
    let (batch, _, _) = build_batch(&drawing, crs, &Limits::default())?;
    Ok(batch.num_rows())
}

#[cfg(test)]
mod tests {
    use super::*;
    use plenora_core::contract::GeometryType;
    use plenora_core::crs::CrsResolution;
    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;

    fn resolved_wgs84() -> ResolvedCrs {
        ResolvedCrs {
            id: Some("EPSG:4326".to_owned()),
            kind: CrsKind::Geographic,
            definition: Some(WGS84_ESRI_WKT.to_owned()),
        }
    }

    fn wkb(value: WkbValue, dimensions: CoordinateDimensions) -> Vec<u8> {
        encode_wkb(
            &WkbGeometry {
                value,
                dimensions,
                srid: None,
            },
            WkbFlavor::Iso,
        )
        .unwrap()
    }

    fn request() -> ReadRequest {
        ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            batch_target: BatchTarget::default(),
        }
    }

    #[test]
    fn write_then_read_xyz_and_embedded_crs_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.dxf");
        let wkb1 = wkb(
            WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: Some(7.5),
                m: None,
            }),
            CoordinateDimensions::Xyz,
        );
        let wkb2 = wkb(
            WkbValue::LineString(vec![
                WkbCoordinate {
                    x: 3.0,
                    y: 4.0,
                    z: Some(8.0),
                    m: None,
                },
                WkbCoordinate {
                    x: 5.0,
                    y: 6.0,
                    z: Some(9.0),
                    m: None,
                },
            ]),
            CoordinateDimensions::Xyz,
        );
        let mut geometry_contract = GeometryColumnContract::wkb_xy(
            FieldId(0),
            GEOMETRY,
            CrsResolution::resolved(resolved_wgs84()),
            true,
        );
        geometry_contract.dimensions = CoordinateDimensions::Xyz;
        geometry_contract.geometry_types = vec![GeometryType::Point, GeometryType::LineString];
        let field = with_geometry_contract_metadata(
            &geometry_field(GEOMETRY, "EPSG:4326"),
            &geometry_contract,
        );
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            field,
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
                    geometry: Some(geometry_contract),
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .unwrap();
        let planned_fidelity = w.fidelity_assessment();
        assert_eq!(
            planned_fidelity.level,
            plenora_io_core::Fidelity::Approximating
        );
        assert!(planned_fidelity
            .reasons
            .iter()
            .any(|reason| { reason.code == plenora_io_core::FidelityReasonCode::AttributeLoss }));
        w.write(&batch).unwrap();
        let published = w.finish().unwrap();
        // "val" non è rappresentabile in DXF -> dichiarato come perdita (Approximating).
        assert!(!published.loss.is_empty());
        assert_eq!(
            published.fidelity.level,
            plenora_io_core::Fidelity::Approximating
        );
        assert!(published
            .fidelity
            .reasons
            .iter()
            .any(|reason| reason.detail.contains("occorrenze")));

        let ds = driver
            .open(Source::Path(out), &ReadOptions::default())
            .unwrap();
        assert_eq!(
            ds.fidelity_assessment().level,
            plenora_io_core::Fidelity::Approximating
        );
        let output_geometry = ds.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(output_geometry.dimensions, CoordinateDimensions::Xyz);
        assert_eq!(output_geometry.crs.id(), Some("EPSG:4326"));
        let mut r = ds.open_layer_reader(&request()).unwrap();
        let rb = r.next_batch().unwrap().unwrap();
        assert_eq!(rb.num_rows(), 2);
        let geometry = rb.column(0).as_any().downcast_ref::<BinaryArray>().unwrap();
        let decoded: Vec<WkbGeometry> = (0..geometry.len())
            .map(|index| decode_wkb(geometry.value(index), &WkbLimits::default()).unwrap())
            .collect();
        assert!(decoded
            .iter()
            .all(|value| value.dimensions == CoordinateDimensions::Xyz));
        assert!(decoded.iter().any(|value| matches!(
            &value.value,
            WkbValue::Point(point) if point.z == Some(7.5)
        )));
        assert!(decoded.iter().any(|value| matches!(
            &value.value,
            WkbValue::LineString(points)
                if points.first().and_then(|point| point.z) == Some(8.0)
                    && points.last().and_then(|point| point.z) == Some(9.0)
        )));
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
        let (batch, _loss, _contract) =
            build_batch(&drawing, resolved_wgs84(), &Limits::default()).unwrap();

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
        let mut found = false;
        for i in 0..batch.num_rows() {
            if types.value(i) == "LINE" {
                if let WkbValue::LineString(points) =
                    decode_wkb(geom.value(i), &WkbLimits::default())
                        .unwrap()
                        .value
                {
                    assert!((points[0].x - 10.0).abs() < 1e-6 && (points[0].y - 5.0).abs() < 1e-6);
                    assert!((points[1].x - 11.0).abs() < 1e-6 && (points[1].y - 5.0).abs() < 1e-6);
                    assert_eq!(layers.value(i), "MYLAYER");
                    found = true;
                }
            }
        }
        assert!(found, "la LINE del blocco esploso non è stata trovata");
    }

    #[test]
    fn missing_geodata_requires_explicit_assumption() {
        let drawing = Drawing::new();
        let error = resolve_dxf_crs(&drawing, &ReadOptions::default()).unwrap_err();
        assert!(error.to_string().contains("assume-crs"));

        let resolved = resolve_dxf_crs(
            &drawing,
            &ReadOptions {
                assume_crs: Some("EPSG:3857".to_owned()),
                ..ReadOptions::default()
            },
        )
        .unwrap();
        assert_eq!(resolved.id.as_deref(), Some("EPSG:3857"));
    }

    #[test]
    fn geodata_epsg_is_resolved_without_fallback() {
        let mut drawing = Drawing::new();
        drawing.add_object(Object::new(ObjectType::GeoData(GeoData {
            coordinate_system_definition: WGS84_ESRI_WKT.to_owned(),
            ..Default::default()
        })));
        let resolved = resolve_dxf_crs(&drawing, &ReadOptions::default()).unwrap();
        assert_eq!(resolved.id.as_deref(), Some("EPSG:4326"));
        assert_eq!(resolved.kind, CrsKind::Geographic);
        assert_eq!(resolved.definition.as_deref(), Some(WGS84_ESRI_WKT));
    }

    #[test]
    fn fuzz_entrypoint_accepts_minimal_ascii_dxf() {
        let dxf = b"0\nSECTION\n2\nENTITIES\n0\nPOINT\n10\n1\n20\n2\n30\n3\n0\nENDSEC\n0\nEOF\n";
        assert_eq!(__fuzz_read_dxf(dxf).unwrap(), 1);
    }

    #[test]
    fn read_limits_are_enforced_before_batch_creation() {
        let mut drawing = Drawing::new();
        drawing.add_entity(Entity::new(EntityType::ModelPoint(ModelPoint {
            location: DxfPoint::new(1.0, 2.0, 3.0),
            ..Default::default()
        })));
        let row_error = build_batch(
            &drawing,
            resolved_wgs84(),
            &Limits {
                max_rows: 0,
                ..Limits::default()
            },
        )
        .unwrap_err();
        assert!(matches!(row_error, PlenoraError::LimitExceeded(_)));

        let column_error = build_batch(
            &drawing,
            resolved_wgs84(),
            &Limits {
                max_columns: 3,
                ..Limits::default()
            },
        )
        .unwrap_err();
        assert!(matches!(column_error, PlenoraError::LimitExceeded(_)));
    }

    #[test]
    fn unsupported_m_is_rejected_before_output_creation() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("m.dxf");
        let mut geometry = GeometryColumnContract::wkb_xy(
            FieldId(0),
            GEOMETRY,
            CrsResolution::resolved(resolved_wgs84()),
            true,
        );
        geometry.dimensions = CoordinateDimensions::Xym;
        geometry.geometry_types = vec![GeometryType::Point];
        let schema: SchemaRef = Arc::new(Schema::new(vec![with_geometry_contract_metadata(
            &geometry_field(GEOMETRY, "EPSG:4326"),
            &geometry,
        )]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "m".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry),
                },
            }],
        };

        assert!(DxfDriver
            .create(Sink::Path(output.clone()), &plan, &WriteOptions::default())
            .is_err());
        assert!(!output.exists());
    }

    #[test]
    fn missing_geometry_contract_is_rejected_before_output_creation() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("missing-crs.dxf");
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "empty".to_owned(),
                contract: DataContract {
                    schema: Arc::new(Schema::empty()),
                    geometry: None,
                },
            }],
        };
        assert!(DxfDriver
            .create(Sink::Path(output.clone()), &plan, &WriteOptions::default())
            .is_err());
        assert!(!output.exists());
    }
}
