//! driver-shp — Shapefile ⇄ RecordBatch. Le shape diventano WKB `geoarrow.wkb`;
//! il dbf fornisce gli attributi; il `.prj` (o `assume_crs`) il CRS.
//!
//! Scrittura (Fase 2B): capability-check fail-closed (ADR-IO 3) — nomi campo dbf
//! ≤10 char (imposto da `FieldName`), tipo geometria unico per file (imposto da
//! shapefile). Publish **multi-file** del set `.shp/.shx/.dbf/.prj` come
//! `LooseShapefileSet` (ADR-IO 2): scrittura in staging dir, poi rename ordinato
//! dei singoli file (il `.shp` per ultimo → la sua presenza segnala il set
//! completo); atomicità dichiarata più debole del dir-dataset. `.prj` scritto se
//! c'è una definizione WKT o per WGS84; nessuna riproiezione (ADR-IO 4).
#![forbid(unsafe_code)]

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Arc;

use arrow_array::builder::{
    BinaryBuilder, BooleanBuilder, Float64Builder, Int64Builder, StringBuilder,
};
use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use serde_json::Value as JsonValue;
use shapefile::dbase::{FieldValue, Record, TableWriterBuilder};
use shapefile::{Shape, Writer};

use driver_common::{geometry_field, json_from_array, ColType};
use plenora_core::contract::{DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId};
use plenora_core::crs::{CrsKind, ResolvedCrs};
use plenora_core::geometry::{is_geometry_field, GEO_CRS_KEY};
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
use plenora_io_core::publish::PublishOutcome;
use plenora_io_core::request::ReadRequest;
use plenora_io_core::WritePlan;

const GEOMETRY: &str = "geometry";

/// WKT standard per WGS84 (accettato da GDAL), usato per il `.prj` quando la
/// sorgente dà solo il codice autorità e non una definizione WKT.
const WGS84_WKT: &str = "GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]";

fn err(reason: impl Into<String>) -> PlenoraError {
    PlenoraError::Format {
        driver: "shp",
        reason: reason.into(),
    }
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "shp",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingSequential,
    write_mode: Some(WriteMode::Streaming),
    multi_layer: false,
    multi_file: true, // .shp/.shx/.dbf/.prj
    reader_concurrency: ReaderConcurrency::MultipleIndependentReaders,
    crs_handling: CrsHandling::Embedded,
    fidelity_class: Fidelity::Conditional,
    runtime: Runtime::PureRust,
    semantic_version: 1,
    driver_version: 3,
    descriptor_version: 1,
};

pub struct ShpDriver;

impl FormatDriver for ShpDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let Source::Path(path) = source;
        let crs = resolve_crs(&path, opts)?;
        // Pass 1: inferenza schema (nomi + tipi) dai record, a RAM O(ncol).
        let cols = infer_shp_schema(&path)?;
        let mut fields = vec![geometry_field(GEOMETRY, crs.id.as_deref().unwrap_or("unknown"))];
        for (n, ct) in &cols {
            fields.push(Field::new(n, coltype_to_dt(*ct), true));
        }
        let schema: SchemaRef = Arc::new(Schema::new(fields));
        let contract = DataContract {
            schema: schema.clone(),
            geometry: Some(GeometryColumnContract {
                field_id: FieldId(0),
                name: GEOMETRY.to_owned(),
                crs: crs.clone(),
                nullable: true,
            }),
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(Box::new(ShpDataset {
            path,
            schema,
            cols,
            layers: vec![LayerContract {
                id: LayerId(0),
                name,
                contract,
            }],
        }))
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        let Sink::Path(dest) = sink;
        if !dest
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("shp"))
        {
            return Err(PlenoraError::Unsupported(
                "l'output deve avere estensione .shp".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraError::Unsupported(
                "Shapefile: un solo layer per file".to_owned(),
            ));
        }
        // no-clobber sull'intero set.
        for ext in ["shp", "shx", "dbf", "prj"] {
            let sib = dest.with_extension(ext);
            if sib.exists() {
                return Err(PlenoraError::OutputExists(sib.display().to_string()));
            }
        }

        let layer = &plan.layers[0];
        let schema = &layer.contract.schema;
        let geom_idx = geometry_index(schema)
            .ok_or_else(|| err("il contratto non ha una colonna geometria geoarrow.wkb"))?;

        // Capability-check (ADR-IO 3): costruisce il dbf, fail-closed sui nomi.
        let mut table = TableWriterBuilder::new();
        let mut attrs: Vec<(usize, String, DbfKind)> = Vec::new();
        for (i, f) in schema.fields().iter().enumerate() {
            if i == geom_idx {
                continue;
            }
            let fname = shapefile::dbase::FieldName::try_from(f.name().as_str()).map_err(|_| {
                PlenoraError::Unsupported(format!(
                    "nome campo '{}' non valido per dbf (max 10 caratteri ASCII)",
                    f.name()
                ))
            })?;
            let kind = DbfKind::from(f.data_type());
            table = match kind {
                DbfKind::Char => table.add_character_field(fname, 254),
                DbfKind::Int => table.add_numeric_field(fname, 18, 0),
                DbfKind::Float => table.add_numeric_field(fname, 20, 8),
                DbfKind::Logical => table.add_logical_field(fname),
            };
            attrs.push((i, f.name().clone(), kind));
        }

        let parent = dest
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let staging = tempfile::Builder::new().tempdir_in(&parent)?;
        let shp_path = staging.path().join("data.shp");
        let writer =
            Writer::from_path(&shp_path, table).map_err(|e| err(format!("creazione shapefile: {e}")))?;

        Ok(Box::new(ShpWriter {
            staging: Some(staging),
            writer: Some(writer),
            dest,
            durable: opts.durable,
            attrs,
            geom_idx,
            prj: resolve_prj(layer, schema, geom_idx),
            shape_type: None,
        }))
    }
}

// --- lettura streaming -----------------------------------------------------

struct ShpDataset {
    path: PathBuf,
    schema: SchemaRef,
    cols: Vec<(String, ColType)>,
    layers: Vec<LayerContract>,
}

impl OpenDatasetHandle for ShpDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        let batch_size = request.batch_target.max_rows.max(1);
        let rx = spawn_parser(
            self.path.clone(),
            self.schema.clone(),
            self.cols.clone(),
            batch_size,
        );
        Ok(Box::new(ShpReader {
            rx,
            layer: self.layers[0].clone(),
        }))
    }
}

struct ShpReader {
    rx: Receiver<std::result::Result<RecordBatch, String>>,
    layer: LayerContract,
}

impl LayerReader for ShpReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }
    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        match self.rx.recv() {
            Ok(Ok(b)) => Ok(Some(b)),
            Ok(Err(e)) => Err(err(e)),
            Err(_) => Ok(None),
        }
    }
}

// --- scrittura -------------------------------------------------------------

#[derive(Clone, Copy)]
enum DbfKind {
    Char,
    Int,
    Float,
    Logical,
}

impl DbfKind {
    fn from(dt: &arrow_schema::DataType) -> Self {
        use arrow_schema::DataType as D;
        match dt {
            D::Int8 | D::Int16 | D::Int32 | D::Int64 | D::UInt8 | D::UInt16 | D::UInt32
            | D::UInt64 => DbfKind::Int,
            D::Float16 | D::Float32 | D::Float64 => DbfKind::Float,
            D::Boolean => DbfKind::Logical,
            _ => DbfKind::Char,
        }
    }
}

struct ShpWriter {
    staging: Option<tempfile::TempDir>,
    writer: Option<Writer<BufWriter<File>>>,
    dest: PathBuf,
    durable: bool,
    attrs: Vec<(usize, String, DbfKind)>,
    geom_idx: usize,
    prj: Option<String>,
    shape_type: Option<&'static str>,
}

impl FormatWriter for ShpWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let geom_col = batch
            .column(self.geom_idx)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| err("colonna geometria non binaria"))?;
        let limits = WkbLimits::default();
        let w = self.writer.as_mut().ok_or_else(|| err("writer chiuso"))?;
        let mut st = self.shape_type;
        for row in 0..batch.num_rows() {
            let shape = if geom_col.is_null(row) {
                Shape::NullShape
            } else {
                let g = from_wkb(geom_col.value(row), &limits)?;
                Shape::try_from(g).map_err(|_| {
                    err("geometria non rappresentabile in Shapefile (es. GeometryCollection)")
                })?
            };
            // Capability-check (ADR-IO 3): un unico tipo di geometria per file.
            let tag = shape_tag(&shape);
            if tag == "unsupported" {
                return Err(err("tipo geometria non supportato da Shapefile (M/Z)"));
            }
            if !tag.is_empty() {
                match st {
                    None => st = Some(tag),
                    Some(e) if e != tag => {
                        return Err(err(format!(
                            "Shapefile richiede un unico tipo di geometria per file (trovati '{e}' e '{tag}')"
                        )))
                    }
                    _ => {}
                }
            }
            let mut rec = Record::default();
            for (col, name, kind) in &self.attrs {
                rec.insert(name.clone(), cell_to_field(batch.column(*col), row, *kind));
            }
            write_shape(w, shape, &rec)?;
        }
        self.shape_type = st;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        // Finalizza .shp/.shx/.dbf (header + bounding box) rilasciando il writer.
        let w = self.writer.take().ok_or_else(|| err("writer già chiuso"))?;
        drop(w);
        let staging = self.staging.take().ok_or_else(|| err("staging mancante"))?;

        if let Some(wkt) = &self.prj {
            std::fs::write(staging.path().join("data.prj"), wkt)?;
        }

        // Publish LooseShapefileSet: rename ordinato, .shp per ultimo.
        let mut bytes = 0u64;
        for ext in ["dbf", "shx", "prj", "shp"] {
            let src = staging.path().join(format!("data.{ext}"));
            if !src.exists() {
                continue;
            }
            let d = self.dest.with_extension(ext);
            if d.exists() {
                return Err(PlenoraError::OutputExists(d.display().to_string()));
            }
            if self.durable {
                if let Ok(f) = File::open(&src) {
                    let _ = f.sync_all();
                }
            }
            bytes += std::fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
            std::fs::rename(&src, &d)?;
        }
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            outcome: PublishOutcome::Published,
        })
    }
}

fn shape_tag(s: &Shape) -> &'static str {
    match s {
        Shape::Point(_) => "point",
        Shape::Polyline(_) => "polyline",
        Shape::Polygon(_) => "polygon",
        Shape::Multipoint(_) => "multipoint",
        Shape::NullShape => "",
        _ => "unsupported",
    }
}

/// Scrive la shape come tipo ESRI concreto (l'enum `Shape` non è `EsriShape`).
fn write_shape(w: &mut Writer<BufWriter<File>>, shape: Shape, rec: &Record) -> Result<()> {
    let me = |e| err(format!("scrittura record shapefile: {e}"));
    match shape {
        Shape::Point(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::Polyline(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::Polygon(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::Multipoint(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::NullShape => Err(err("geometria nulla non supportata in scrittura Shapefile (v1)")),
        _ => Err(err("tipo geometria non supportato da Shapefile (M/Z)")),
    }
}

fn cell_to_field(array: &ArrayRef, row: usize, kind: DbfKind) -> FieldValue {
    let v = json_from_array(array, row);
    match kind {
        DbfKind::Char => FieldValue::Character(match v {
            JsonValue::Null => None,
            JsonValue::String(s) => Some(s),
            other => Some(other.to_string()),
        }),
        DbfKind::Int | DbfKind::Float => FieldValue::Numeric(v.as_f64()),
        DbfKind::Logical => FieldValue::Logical(v.as_bool()),
    }
}

fn geometry_index(schema: &Schema) -> Option<usize> {
    schema.fields().iter().position(|f| is_geometry_field(f))
}

fn wkt_for_id(id: Option<&str>) -> Option<String> {
    match id {
        Some("EPSG:4326") | Some("OGC:CRS84") => Some(WGS84_WKT.to_owned()),
        _ => None,
    }
}

fn resolve_prj(layer: &plenora_io_core::WriteLayer, schema: &Schema, geom_idx: usize) -> Option<String> {
    if let Some(g) = &layer.contract.geometry {
        if let Some(def) = &g.crs.definition {
            return Some(def.clone());
        }
        if let Some(wkt) = wkt_for_id(g.crs.id.as_deref()) {
            return Some(wkt);
        }
    }
    let id = schema.field(geom_idx).metadata().get(GEO_CRS_KEY).map(String::as_str);
    wkt_for_id(id)
}

// --- lettura: helpers ------------------------------------------------------

fn resolve_crs(path: &Path, opts: &ReadOptions) -> Result<ResolvedCrs> {
    let prj = path.with_extension("prj");
    if let Ok(wkt) = std::fs::read_to_string(&prj) {
        return Ok(ResolvedCrs {
            id: opts.assume_crs.clone(),
            kind: CrsKind::Unknown,
            definition: Some(wkt),
        });
    }
    match &opts.assume_crs {
        Some(id) => Ok(ResolvedCrs {
            id: Some(id.clone()),
            kind: CrsKind::Unknown,
            definition: None,
        }),
        None => Err(PlenoraError::Crs(
            "Shapefile senza .prj: fornire --assume-crs".to_owned(),
        )),
    }
}

fn coltype_to_dt(ct: ColType) -> DataType {
    match ct {
        ColType::Integer => DataType::Int64,
        ColType::Number => DataType::Float64,
        ColType::Boolean => DataType::Boolean,
        ColType::Text => DataType::Utf8,
    }
}

#[derive(Clone, Copy)]
struct Acc {
    any: bool,
    all_int: bool,
    all_num: bool,
    all_bool: bool,
}

impl Default for Acc {
    fn default() -> Self {
        Acc { any: false, all_int: true, all_num: true, all_bool: true }
    }
}

impl Acc {
    fn observe(&mut self, class: u8) {
        match class {
            0 => {}
            1 => {
                self.any = true;
                self.all_bool = false;
            }
            2 => {
                self.any = true;
                self.all_int = false;
                self.all_bool = false;
            }
            3 => {
                self.any = true;
                self.all_int = false;
                self.all_num = false;
            }
            _ => {
                self.any = true;
                self.all_int = false;
                self.all_num = false;
                self.all_bool = false;
            }
        }
    }
    fn coltype(&self) -> ColType {
        if !self.any {
            ColType::Text
        } else if self.all_int {
            ColType::Integer
        } else if self.all_bool {
            ColType::Boolean
        } else if self.all_num {
            ColType::Number
        } else {
            ColType::Text
        }
    }
}

/// Classe dbf per l'inferenza (Numeric/Double/Float=numero, Integer=int).
fn classify(v: &FieldValue) -> u8 {
    match v {
        FieldValue::Integer(_) => 1,
        FieldValue::Numeric(Some(_)) | FieldValue::Double(_) | FieldValue::Float(Some(_)) => 2,
        FieldValue::Logical(Some(_)) => 3,
        FieldValue::Character(Some(_)) | FieldValue::Date(Some(_)) => 4,
        _ => 0,
    }
}

/// Pass 1: nomi campo (in ordine dbf) + tipo inferito, a RAM O(ncol).
fn infer_shp_schema(path: &Path) -> Result<Vec<(String, ColType)>> {
    let mut reader =
        shapefile::Reader::from_path(path).map_err(|e| err(format!("apertura shapefile: {e}")))?;
    let mut order: Vec<String> = Vec::new();
    let mut accs: std::collections::HashMap<String, Acc> = std::collections::HashMap::new();
    for pair in reader.iter_shapes_and_records() {
        let (_, record) = pair.map_err(|e| err(format!("record shapefile: {e}")))?;
        for (name, value) in record {
            if !accs.contains_key(&name) {
                order.push(name.clone());
                accs.insert(name.clone(), Acc::default());
            }
            accs.get_mut(&name).unwrap().observe(classify(&value));
        }
    }
    Ok(order
        .into_iter()
        .map(|n| {
            let ct = accs[&n].coltype();
            (n, ct)
        })
        .collect())
}

/// Pass 2: thread che scorre i record e produce batch da `batch_size` righe.
fn spawn_parser(
    path: PathBuf,
    schema: SchemaRef,
    cols: Vec<(String, ColType)>,
    batch_size: usize,
) -> Receiver<std::result::Result<RecordBatch, String>> {
    let (tx, rx) = sync_channel::<std::result::Result<RecordBatch, String>>(2);
    std::thread::spawn(move || {
        let run = || -> std::result::Result<(), String> {
            let mut reader = shapefile::Reader::from_path(&path).map_err(|e| e.to_string())?;
            let mut geom = BinaryBuilder::new();
            let mut builders: Vec<ShpColBuilder> =
                cols.iter().map(|(_, ct)| ShpColBuilder::new(*ct)).collect();
            let mut n = 0usize;
            for pair in reader.iter_shapes_and_records() {
                let (shape, record) = pair.map_err(|e| e.to_string())?;
                match geo_types::Geometry::<f64>::try_from(shape) {
                    Ok(g) => geom.append_value(to_wkb(&g).map_err(|e| e.to_string())?),
                    Err(_) => geom.append_null(),
                }
                // Lookup per nome (l'ordine di iterazione del Record non è garantito).
                let map: std::collections::HashMap<String, FieldValue> =
                    record.into_iter().collect();
                for (k, (name, _)) in cols.iter().enumerate() {
                    builders[k].append(map.get(name));
                }
                n += 1;
                if n >= batch_size {
                    let batch = finish_batch(&schema, &mut geom, &mut builders)?;
                    if tx.send(Ok(batch)).is_err() {
                        return Ok(());
                    }
                    n = 0;
                }
            }
            if n > 0 {
                let batch = finish_batch(&schema, &mut geom, &mut builders)?;
                let _ = tx.send(Ok(batch));
            }
            Ok(())
        };
        if let Err(e) = run() {
            let _ = tx.send(Err(e));
        }
    });
    rx
}

fn finish_batch(
    schema: &SchemaRef,
    geom: &mut BinaryBuilder,
    builders: &mut [ShpColBuilder],
) -> std::result::Result<RecordBatch, String> {
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(1 + builders.len());
    arrays.push(Arc::new(geom.finish()));
    for b in builders.iter_mut() {
        arrays.push(b.finish());
    }
    RecordBatch::try_new(schema.clone(), arrays).map_err(|e| format!("batch: {e}"))
}

enum ShpColBuilder {
    I64(Int64Builder),
    F64(Float64Builder),
    Bool(BooleanBuilder),
    Str(StringBuilder),
}

impl ShpColBuilder {
    fn new(ct: ColType) -> Self {
        match ct {
            ColType::Integer => ShpColBuilder::I64(Int64Builder::new()),
            ColType::Number => ShpColBuilder::F64(Float64Builder::new()),
            ColType::Boolean => ShpColBuilder::Bool(BooleanBuilder::new()),
            ColType::Text => ShpColBuilder::Str(StringBuilder::new()),
        }
    }
    fn append(&mut self, v: Option<&FieldValue>) {
        match self {
            ShpColBuilder::I64(b) => b.append_option(v.and_then(fv_i64)),
            ShpColBuilder::F64(b) => b.append_option(v.and_then(fv_f64)),
            ShpColBuilder::Bool(b) => b.append_option(v.and_then(fv_bool)),
            ShpColBuilder::Str(b) => match v.and_then(fv_string) {
                Some(s) => b.append_value(s),
                None => b.append_null(),
            },
        }
    }
    fn finish(&mut self) -> ArrayRef {
        match self {
            ShpColBuilder::I64(b) => Arc::new(b.finish()),
            ShpColBuilder::F64(b) => Arc::new(b.finish()),
            ShpColBuilder::Bool(b) => Arc::new(b.finish()),
            ShpColBuilder::Str(b) => Arc::new(b.finish()),
        }
    }
}

fn fv_i64(v: &FieldValue) -> Option<i64> {
    match v {
        FieldValue::Integer(i) => Some(*i as i64),
        FieldValue::Numeric(Some(n)) => Some(*n as i64),
        FieldValue::Double(d) => Some(*d as i64),
        FieldValue::Float(Some(f)) => Some(*f as i64),
        _ => None,
    }
}

fn fv_f64(v: &FieldValue) -> Option<f64> {
    match v {
        FieldValue::Numeric(Some(n)) => Some(*n),
        FieldValue::Double(d) => Some(*d),
        FieldValue::Float(Some(f)) => Some(*f as f64),
        FieldValue::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

fn fv_bool(v: &FieldValue) -> Option<bool> {
    match v {
        FieldValue::Logical(Some(b)) => Some(*b),
        _ => None,
    }
}

fn fv_string(v: &FieldValue) -> Option<String> {
    match v {
        FieldValue::Character(Some(s)) => Some(s.clone()),
        FieldValue::Date(Some(d)) => {
            Some(format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()))
        }
        FieldValue::Integer(i) => Some(i.to_string()),
        FieldValue::Numeric(Some(n)) => Some(n.to_string()),
        FieldValue::Double(d) => Some(d.to_string()),
        FieldValue::Float(Some(f)) => Some(f.to_string()),
        FieldValue::Logical(Some(b)) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;

    fn read_opts() -> ReadOptions {
        ReadOptions {
            assume_crs: Some("EPSG:4326".to_owned()),
            format_options: Default::default(),
        }
    }

    fn req() -> ReadRequest {
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
    fn write_then_read_round_trip() {
        use arrow_array::{Int64Array, StringArray};
        use arrow_schema::DataType;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("pts.shp");

        let wkb1 = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(12.5, 45.9))).unwrap();
        let wkb2 = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(9.19, 45.46))).unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("nome", DataType::Utf8, true),
            Field::new("pop", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![
                    Some(wkb1.as_slice()),
                    Some(wkb2.as_slice()),
                ])),
                Arc::new(StringArray::from(vec!["Roma", "Milano"])),
                Arc::new(Int64Array::from(vec![2800000i64, 1400000])),
            ],
        )
        .unwrap();

        let driver = ShpDriver;
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

        // il set è stato pubblicato
        assert!(out.exists());
        assert!(out.with_extension("dbf").exists());
        assert!(out.with_extension("prj").exists());

        // rilettura
        let ds = driver.open(Source::Path(out), &read_opts()).unwrap();
        let mut r = ds.open_layer_reader(&req()).unwrap();
        let rb = r.next_batch().unwrap().unwrap();
        assert_eq!(rb.num_rows(), 2);
        let nome = rb
            .column_by_name("nome")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(nome.value(0), "Roma");
    }

    #[test]
    fn rejects_long_field_name() {
        use arrow_schema::DataType;
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("nome_campo_troppo_lungo", DataType::Utf8, true),
        ]));
        let driver = ShpDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let dir = tempfile::tempdir().unwrap();
        let e = driver
            .create(
                Sink::Path(dir.path().join("x.shp")),
                &plan,
                &WriteOptions::default(),
            )
            .map(|_| ())
            .unwrap_err();
        assert!(matches!(e, PlenoraError::Unsupported(_)));
    }

    #[test]
    fn streams_multiple_batches() {
        use arrow_array::Int64Array;
        use arrow_schema::DataType;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("many.shp");
        let wkb: Vec<Vec<u8>> = (0..10)
            .map(|i| {
                to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(i as f64, i as f64)))
                    .unwrap()
            })
            .collect();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("id", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(
                    wkb.iter().map(|w| Some(w.as_slice())).collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from((0..10i64).collect::<Vec<_>>())),
            ],
        )
        .unwrap();

        let driver = ShpDriver;
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

        let ds = driver.open(Source::Path(out), &read_opts()).unwrap();
        let req = ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            batch_target: BatchTarget {
                target_bytes: 8 * 1024 * 1024,
                max_rows: 4,
            },
        };
        let mut r = ds.open_layer_reader(&req).unwrap();
        let (mut total, mut batches) = (0, 0);
        while let Some(b) = r.next_batch().unwrap() {
            total += b.num_rows();
            batches += 1;
        }
        assert_eq!(total, 10);
        assert!(batches >= 3, "atteso streaming multi-batch, avuti {batches}");
    }
}
