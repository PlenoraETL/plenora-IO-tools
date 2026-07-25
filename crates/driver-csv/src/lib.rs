//! driver-csv — CSV ⇄ RecordBatch. La geometria è dichiarata via
//! `format_options`: `x_column`+`y_column` (Point) oppure `wkt_column` (qualsiasi
//! tipo). CSV non porta CRS: `assume_crs` è obbligatorio (ADR-IO 4).
//!
//! Lettura **streaming** (Fase 2A): righe scorse via `csv::StringRecord` riusato
//! (i campi sono `&str`, niente String per cella). Due passate: pass-1 (`open`)
//! inferisce i tipi colonna a RAM O(1) sondando le celle (nessuna allocazione);
//! pass-2 è un thread che produce RecordBatch da `batch_target` righe via canale
//! con backpressure → memoria O(batch). Geometria diretta a WKB, attributi in
//! builder tipizzati (niente intermedio `serde_json::Value`). Scrittura streaming
//! per righe (niente buffering dei batch).
#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Arc;

use arrow_array::builder::{
    BinaryBuilder, BooleanBuilder, Float64Builder, Int64Builder, StringBuilder,
};
use arrow_array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use serde_json::Value as JsonValue;
use wkt::{ToWkt, TryFromWkt};

use driver_common::{geometry_field, json_from_array, ColType};
use plenora_core::contract::{DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId};
use plenora_core::crs::{CrsKind, ResolvedCrs};
use plenora_core::geometry::is_geometry_field;
use plenora_core::limits::WkbLimits;
use plenora_core::wkb::{from_wkb, to_wkb_into};
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
        driver: "csv",
        reason: reason.into(),
    }
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "csv",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingSequential,
    write_mode: Some(WriteMode::Streaming),
    multi_layer: false,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::MultipleIndependentReaders,
    crs_handling: CrsHandling::None,
    fidelity_class: Fidelity::Conditional,
    runtime: Runtime::PureRust,
    semantic_version: 1,
    driver_version: 2,
    descriptor_version: 1,
};

pub struct CsvDriver;

fn delimiter(opts: &std::collections::BTreeMap<String, String>) -> u8 {
    opts.get("delimiter")
        .and_then(|s| s.bytes().next())
        .unwrap_or(b',')
}

fn csv_reader(path: &Path, delim: u8) -> Result<csv::Reader<File>> {
    csv::ReaderBuilder::new()
        .delimiter(delim)
        .has_headers(true) // salta l'intestazione automaticamente
        .flexible(true)
        .from_path(path)
        .map_err(|e| err(format!("apertura CSV: {e}")))
}

#[derive(Clone, Copy)]
enum GeomSpec {
    Wkt(usize),
    Xy(usize, usize),
}

impl FormatDriver for CsvDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let Source::Path(path) = source;
        let delim = delimiter(&opts.format_options);
        let crs = opts.assume_crs.clone().ok_or_else(|| {
            PlenoraError::Crs("CSV con geometria richiede --assume-crs".to_owned())
        })?;

        // Intestazione (nomi colonna).
        let headers: Vec<String> = {
            let mut rdr = csv::ReaderBuilder::new()
                .delimiter(delim)
                .has_headers(false)
                .flexible(true)
                .from_path(&path)
                .map_err(|e| err(format!("apertura CSV: {e}")))?;
            let mut first = csv::StringRecord::new();
            if !rdr
                .read_record(&mut first)
                .map_err(|e| err(format!("CSV non valido: {e}")))?
            {
                return Err(err("CSV vuoto"));
            }
            first.iter().map(str::to_owned).collect()
        };

        let idx = |name: &str| headers.iter().position(|h| h == name);
        let (geom, geom_cols): (GeomSpec, HashSet<usize>) =
            if let Some(w) = opts.format_options.get("wkt_column") {
                let wi = idx(w).ok_or_else(|| err(format!("colonna WKT '{w}' assente")))?;
                (GeomSpec::Wkt(wi), HashSet::from([wi]))
            } else if let (Some(x), Some(y)) =
                (opts.format_options.get("x_column"), opts.format_options.get("y_column"))
            {
                let xi = idx(x).ok_or_else(|| err(format!("colonna X '{x}' assente")))?;
                let yi = idx(y).ok_or_else(|| err(format!("colonna Y '{y}' assente")))?;
                (GeomSpec::Xy(xi, yi), HashSet::from([xi, yi]))
            } else {
                return Err(err(
                    "specificare wkt_column, oppure x_column con y_column, in format_options",
                ));
            };

        // Pass 1: inferenza tipi (RAM O(ncol), nessuna String per cella).
        let attrs = infer_types(&path, delim, &headers, &geom_cols)?;

        let mut fields = vec![geometry_field(GEOMETRY, &crs)];
        for (ci, ct) in &attrs {
            fields.push(Field::new(&headers[*ci], coltype_to_dt(*ct), true));
        }
        let schema: SchemaRef = Arc::new(Schema::new(fields));
        let kind = if crs == "OGC:CRS84" || crs == "EPSG:4326" {
            CrsKind::Geographic
        } else {
            CrsKind::Unknown
        };
        let contract = DataContract {
            schema: schema.clone(),
            geometry: Some(GeometryColumnContract {
                field_id: FieldId(0),
                name: GEOMETRY.to_owned(),
                crs: ResolvedCrs {
                    id: Some(crs.clone()),
                    kind,
                    definition: None,
                },
                nullable: true,
            }),
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(Box::new(CsvDataset {
            path,
            delim,
            geom,
            attrs,
            schema,
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
        let Sink::Path(path) = sink;
        if path.exists() {
            return Err(PlenoraError::OutputExists(path.display().to_string()));
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
        {
            return Err(PlenoraError::Unsupported(
                "l'output deve avere estensione .csv".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraError::Unsupported(
                "CSV: un solo layer per file".to_owned(),
            ));
        }
        let xy = matches!(
            opts.format_options.get("geometry_encoding").map(String::as_str),
            Some("xy")
        );
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let temp = tempfile::NamedTempFile::new_in(&parent)?;
        let writer = csv::WriterBuilder::new()
            .delimiter(delimiter(&opts.format_options))
            .from_writer(temp.reopen()?);
        Ok(Box::new(CsvWriter {
            temp: Some(temp),
            writer: Some(writer),
            path,
            durable: opts.durable,
            xy,
            header_written: false,
        }))
    }
}

// --- lettura streaming -----------------------------------------------------

struct CsvDataset {
    path: PathBuf,
    delim: u8,
    geom: GeomSpec,
    attrs: Vec<(usize, ColType)>,
    schema: SchemaRef,
    layers: Vec<LayerContract>,
}

impl OpenDatasetHandle for CsvDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        let batch_size = request.batch_target.max_rows.max(1);
        let rx = spawn_parser(
            self.path.clone(),
            self.delim,
            self.geom,
            self.attrs.clone(),
            self.schema.clone(),
            batch_size,
        );
        Ok(Box::new(CsvStreamReader {
            rx,
            layer: self.layers[0].clone(),
        }))
    }
}

struct CsvStreamReader {
    rx: Receiver<std::result::Result<RecordBatch, String>>,
    layer: LayerContract,
}

impl LayerReader for CsvStreamReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }
    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        match self.rx.recv() {
            Ok(Ok(batch)) => Ok(Some(batch)),
            Ok(Err(e)) => Err(err(e)),
            Err(_) => Ok(None),
        }
    }
}

/// Classe di una cella per l'inferenza (stessa semantica di `cell_to_json` +
/// `infer_column`): 0 null, 1 int, 2 float, 3 bool, 4 testo.
fn classify(cell: &str) -> u8 {
    let t = cell.trim();
    if t.is_empty() {
        return 0;
    }
    if t.parse::<i64>().is_ok() {
        return 1;
    }
    if t.parse::<f64>().is_ok() {
        return 2;
    }
    if t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("false") {
        return 3;
    }
    4
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
        Acc {
            any: false,
            all_int: true,
            all_num: true,
            all_bool: true,
        }
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

fn coltype_to_dt(ct: ColType) -> DataType {
    match ct {
        ColType::Integer => DataType::Int64,
        ColType::Number => DataType::Float64,
        ColType::Boolean => DataType::Boolean,
        ColType::Text => DataType::Utf8,
    }
}

fn infer_types(
    path: &Path,
    delim: u8,
    headers: &[String],
    geom_cols: &HashSet<usize>,
) -> Result<Vec<(usize, ColType)>> {
    let attr_idx: Vec<usize> = (0..headers.len()).filter(|i| !geom_cols.contains(i)).collect();
    let mut accs = vec![Acc::default(); attr_idx.len()];
    let mut rdr = csv_reader(path, delim)?;
    let mut rec = csv::StringRecord::new();
    while rdr
        .read_record(&mut rec)
        .map_err(|e| err(format!("riga CSV non valida: {e}")))?
    {
        for (j, &ci) in attr_idx.iter().enumerate() {
            accs[j].observe(classify(rec.get(ci).unwrap_or("")));
        }
    }
    Ok(attr_idx
        .into_iter()
        .zip(accs)
        .map(|(ci, a)| (ci, a.coltype()))
        .collect())
}

fn spawn_parser(
    path: PathBuf,
    delim: u8,
    geom: GeomSpec,
    attrs: Vec<(usize, ColType)>,
    schema: SchemaRef,
    batch_size: usize,
) -> Receiver<std::result::Result<RecordBatch, String>> {
    let (tx, rx) = sync_channel::<std::result::Result<RecordBatch, String>>(2);
    std::thread::spawn(move || {
        let run = || -> std::result::Result<(), String> {
            let mut rdr = csv_reader(&path, delim).map_err(|e| e.to_string())?;
            let mut rec = csv::StringRecord::new();
            let mut geom_b = BinaryBuilder::new();
            let mut wkb_buf: Vec<u8> = Vec::new(); // riusato per riga: 0 alloc WKB nel loop
            let mut builders: Vec<ColBuilder> =
                attrs.iter().map(|(_, ct)| ColBuilder::new(*ct)).collect();
            let mut n = 0usize;
            loop {
                let more = rdr
                    .read_record(&mut rec)
                    .map_err(|e| format!("riga CSV non valida: {e}"))?;
                if !more {
                    break;
                }
                append_geometry(&mut geom_b, geom, &rec, &mut wkb_buf)?;
                for (k, (ci, _)) in attrs.iter().enumerate() {
                    builders[k].append(rec.get(*ci).unwrap_or(""));
                }
                n += 1;
                if n >= batch_size {
                    let batch = finish_batch(&schema, &mut geom_b, &mut builders)?;
                    if tx.send(Ok(batch)).is_err() {
                        return Ok(());
                    }
                    n = 0;
                }
            }
            if n > 0 {
                let batch = finish_batch(&schema, &mut geom_b, &mut builders)?;
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

fn append_geometry(
    geom_b: &mut BinaryBuilder,
    geom: GeomSpec,
    rec: &csv::StringRecord,
    buf: &mut Vec<u8>,
) -> std::result::Result<(), String> {
    match geom {
        GeomSpec::Wkt(wi) => {
            let cell = rec.get(wi).unwrap_or("").trim();
            if cell.is_empty() {
                geom_b.append_null();
            } else {
                let g = geo_types::Geometry::<f64>::try_from_wkt_str(cell)
                    .map_err(|e| format!("WKT non valido: {e}"))?;
                to_wkb_into(&g, buf).map_err(|e| e.to_string())?;
                geom_b.append_value(&buf);
            }
        }
        GeomSpec::Xy(xi, yi) => {
            let x = rec.get(xi).unwrap_or("").trim().parse::<f64>();
            let y = rec.get(yi).unwrap_or("").trim().parse::<f64>();
            match (x, y) {
                (Ok(x), Ok(y)) => {
                    let g = geo_types::Geometry::Point(geo_types::Point::new(x, y));
                    to_wkb_into(&g, buf).map_err(|e| e.to_string())?;
                    geom_b.append_value(&buf);
                }
                _ => geom_b.append_null(),
            }
        }
    }
    Ok(())
}

fn finish_batch(
    schema: &SchemaRef,
    geom_b: &mut BinaryBuilder,
    builders: &mut [ColBuilder],
) -> std::result::Result<RecordBatch, String> {
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(1 + builders.len());
    arrays.push(Arc::new(geom_b.finish()));
    for b in builders.iter_mut() {
        arrays.push(b.finish());
    }
    RecordBatch::try_new(schema.clone(), arrays).map_err(|e| format!("record batch: {e}"))
}

enum ColBuilder {
    I64(Int64Builder),
    F64(Float64Builder),
    Bool(BooleanBuilder),
    Str(StringBuilder),
}

impl ColBuilder {
    fn new(ct: ColType) -> Self {
        match ct {
            ColType::Integer => ColBuilder::I64(Int64Builder::new()),
            ColType::Number => ColBuilder::F64(Float64Builder::new()),
            ColType::Boolean => ColBuilder::Bool(BooleanBuilder::new()),
            ColType::Text => ColBuilder::Str(StringBuilder::new()),
        }
    }
    fn append(&mut self, cell: &str) {
        let t = cell.trim();
        match self {
            ColBuilder::I64(b) => b.append_option(if t.is_empty() { None } else { t.parse().ok() }),
            ColBuilder::F64(b) => b.append_option(if t.is_empty() { None } else { t.parse().ok() }),
            ColBuilder::Bool(b) => b.append_option(if t.eq_ignore_ascii_case("true") {
                Some(true)
            } else if t.eq_ignore_ascii_case("false") {
                Some(false)
            } else {
                None
            }),
            ColBuilder::Str(b) => {
                if t.is_empty() {
                    b.append_null();
                } else {
                    b.append_value(cell); // stringa originale (non trimmata)
                }
            }
        }
    }
    fn finish(&mut self) -> ArrayRef {
        match self {
            ColBuilder::I64(b) => Arc::new(b.finish()),
            ColBuilder::F64(b) => Arc::new(b.finish()),
            ColBuilder::Bool(b) => Arc::new(b.finish()),
            ColBuilder::Str(b) => Arc::new(b.finish()),
        }
    }
}

// --- scrittura streaming ---------------------------------------------------

struct CsvWriter {
    temp: Option<tempfile::NamedTempFile>,
    writer: Option<csv::Writer<File>>,
    path: PathBuf,
    durable: bool,
    xy: bool,
    header_written: bool,
}

impl FormatWriter for CsvWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let schema = batch.schema();
        let geom_idx = schema
            .fields()
            .iter()
            .position(|f| is_geometry_field(f))
            .ok_or_else(|| err("nessuna colonna geometria"))?;
        let geom_col = batch
            .column(geom_idx)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| err("colonna geometria non binaria"))?;
        let limits = WkbLimits::default();
        let xy = self.xy;
        let w = self.writer.as_mut().ok_or_else(|| err("writer chiuso"))?;

        if !self.header_written {
            let mut header: Vec<&str> = Vec::new();
            for (i, f) in schema.fields().iter().enumerate() {
                if i != geom_idx {
                    header.push(f.name());
                }
            }
            if xy {
                header.push("x");
                header.push("y");
            } else {
                header.push("geometry");
            }
            w.write_record(&header).map_err(|e| err(e.to_string()))?;
            self.header_written = true;
        }

        // Scrittura per-campo DIRETTA (niente Vec<String> né serde_json::Value
        // per cella): `fbuf` è riusato per formattare numeri/bool.
        let mut fbuf = String::new();
        for row in 0..batch.num_rows() {
            for (i, _) in schema.fields().iter().enumerate() {
                if i != geom_idx {
                    write_cell(w, batch.column(i), row, &mut fbuf).map_err(|e| err(e.to_string()))?;
                }
            }
            if geom_col.is_null(row) {
                w.write_field("").map_err(|e| err(e.to_string()))?;
                if xy {
                    w.write_field("").map_err(|e| err(e.to_string()))?;
                }
            } else {
                let geom = from_wkb(geom_col.value(row), &limits)?;
                if xy {
                    match geom {
                        geo_types::Geometry::Point(p) => {
                            fbuf.clear();
                            let _ = write!(fbuf, "{}", p.x());
                            w.write_field(&fbuf).map_err(|e| err(e.to_string()))?;
                            fbuf.clear();
                            let _ = write!(fbuf, "{}", p.y());
                            w.write_field(&fbuf).map_err(|e| err(e.to_string()))?;
                        }
                        _ => return Err(err("encoding xy richiede geometrie Point")),
                    }
                } else {
                    w.write_field(geom.wkt_string())
                        .map_err(|e| err(e.to_string()))?;
                }
            }
            // Termina il record (dopo i write_field).
            w.write_record(None::<&[u8]>).map_err(|e| err(e.to_string()))?;
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        let mut w = self.writer.take().ok_or_else(|| err("writer già chiuso"))?;
        w.flush().map_err(|e| err(e.to_string()))?;
        drop(w);
        let temp = self.temp.take().ok_or_else(|| err("temp mancante"))?;
        let (bytes, outcome) = publish_file_atomic(temp, &self.path, self.durable)?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            outcome,
        })
    }
}

/// Scrive una cella attributo DIRETTAMENTE nel writer CSV, senza passare per
/// `serde_json::Value`: stringhe dritte (0 alloc), numeri/bool formattati nel
/// buffer riusato `fbuf`. I tipi non comuni ricadono sul convertitore generico.
fn write_cell<W: std::io::Write>(
    w: &mut csv::Writer<W>,
    col: &ArrayRef,
    row: usize,
    fbuf: &mut String,
) -> csv::Result<()> {
    if col.is_null(row) {
        return w.write_field("");
    }
    if let Some(a) = col.as_any().downcast_ref::<StringArray>() {
        w.write_field(a.value(row))
    } else if let Some(a) = col.as_any().downcast_ref::<Int64Array>() {
        fbuf.clear();
        let _ = write!(fbuf, "{}", a.value(row));
        w.write_field(&*fbuf)
    } else if let Some(a) = col.as_any().downcast_ref::<Float64Array>() {
        fbuf.clear();
        let _ = write!(fbuf, "{}", a.value(row));
        w.write_field(&*fbuf)
    } else if let Some(a) = col.as_any().downcast_ref::<BooleanArray>() {
        w.write_field(if a.value(row) { "true" } else { "false" })
    } else {
        // Tipo non comune (Date, ecc.): fallback via il convertitore generico.
        w.write_field(cell_string(&json_from_array(col, row)))
    }
}

fn cell_string(v: &JsonValue) -> String {
    match v {
        JsonValue::Null => String::new(),
        JsonValue::String(s) => s.clone(),
        JsonValue::Bool(b) => b.to_string(),
        JsonValue::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;
    use std::collections::BTreeMap;

    fn opts(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn read_opts(pairs: &[(&str, &str)]) -> ReadOptions {
        ReadOptions {
            assume_crs: Some("EPSG:4326".to_owned()),
            format_options: opts(pairs),
        }
    }

    fn req(max_rows: usize) -> ReadRequest {
        ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            batch_target: BatchTarget {
                target_bytes: 8 * 1024 * 1024,
                max_rows,
            },
        }
    }

    #[test]
    fn round_trip_csv_xy() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.csv");
        std::fs::write(&src, "nome,lon,lat,pop\nA,12.5,45.9,100\nB,9.1,45.4,200\n").unwrap();

        let driver = CsvDriver;
        let ds = driver
            .open(
                Source::Path(src),
                &read_opts(&[("x_column", "lon"), ("y_column", "lat")]),
            )
            .unwrap();
        let geom = ds.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(geom.crs.id.as_deref(), Some("EPSG:4326"));
        let mut reader = ds.open_layer_reader(&req(65_536)).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert!(is_geometry_field(
            &batch.schema().field_with_name("geometry").unwrap().clone()
        ));
        let contract = ds.layers()[0].contract.clone();

        // scrivi come WKT e rileggi
        let out = dir.path().join("out.csv");
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract,
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.contains("POINT"));
        assert!(text.contains("nome"));
    }

    #[test]
    fn streams_multiple_batches() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("many.csv");
        let mut s = String::from("id,geom\n");
        for i in 0..10 {
            s.push_str(&format!("{i},\"POINT ({i} {i})\"\n"));
        }
        std::fs::write(&src, s).unwrap();

        let driver = CsvDriver;
        let ds = driver
            .open(Source::Path(src), &read_opts(&[("wkt_column", "geom")]))
            .unwrap();
        let mut reader = ds.open_layer_reader(&req(4)).unwrap();
        let (mut total, mut batches) = (0, 0);
        while let Some(b) = reader.next_batch().unwrap() {
            total += b.num_rows();
            batches += 1;
        }
        assert_eq!(total, 10);
        assert!(batches >= 3, "atteso streaming multi-batch, avuti {batches}");
    }
}
