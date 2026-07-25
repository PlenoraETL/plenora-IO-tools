//! driver-geoparquet — GeoParquet ⇄ RecordBatch (Fase 1). La geometria è WKB:
//! in lettura la colonna binaria viene ri-etichettata `geoarrow.wkb` + `crs`
//! SENZA decodifica (pass-through, V4); in scrittura si emette il metadato `geo`
//! dal contratto. zstd rifiutato in lettura (build puro-Rust).
#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, Float64Array, LargeBinaryArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use parquet::arrow::arrow_reader::{ParquetRecordBatchReader, ParquetRecordBatchReaderBuilder};
use parquet::arrow::{ArrowWriter, ProjectionMask};
use parquet::basic::Compression;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use parquet::file::statistics::Statistics;

use plenora_core::contract::{
    DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId,
};
use plenora_core::crs::{CrsKind, ResolvedCrs};
use plenora_core::geometry::{ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, GEO_CRS_KEY};
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
use plenora_io_core::request::{Bbox, ProjectionMode, PruningPredicate, ReadRequest};
use plenora_io_core::WritePlan;

fn fmt_err(reason: impl Into<String>) -> PlenoraError {
    PlenoraError::Format {
        driver: "geoparquet",
        reason: reason.into(),
    }
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "geoparquet",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingColumnar,
    write_mode: Some(WriteMode::Streaming),
    multi_layer: false,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::MultipleIndependentReaders, // Parquet è seekable
    crs_handling: CrsHandling::Embedded,
    fidelity_class: Fidelity::Lossless,
    runtime: Runtime::PureRust,
    semantic_version: 1,
    driver_version: 1,
    descriptor_version: 1,
};

pub struct GeoParquetDriver;

impl FormatDriver for GeoParquetDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, _opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let Source::Path(path) = source;
        let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&path)?)
            .map_err(|e| fmt_err(format!("Parquet non valido: {e}")))?;
        reject_zstd(&builder)?;
        let parquet_schema = builder.schema().clone();
        let geo = read_geo_meta(&builder);
        let (geom_name, crs) = resolve_geometry_and_crs(&parquet_schema, geo.as_ref())?;
        let out_schema = retag_schema(&parquet_schema, &geom_name, &crs);
        let has_bbox = BBOX_COLS.iter().all(|n| parquet_schema.index_of(n).is_ok());
        let geom_idx = out_schema
            .index_of(&geom_name)
            .map_err(|e| fmt_err(format!("colonna geometria: {e}")))?;
        let contract = DataContract {
            schema: out_schema.clone(),
            geometry: Some(GeometryColumnContract {
                field_id: FieldId(geom_idx as u32),
                name: geom_name.clone(),
                crs: crs.clone(),
                nullable: out_schema.field(geom_idx).is_nullable(),
            }),
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        let layer = LayerContract {
            id: LayerId(0),
            name,
            contract,
        };
        Ok(Box::new(GeoParquetDataset {
            path,
            out_schema,
            has_bbox,
            layers: vec![layer],
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
            .is_some_and(|e| e.eq_ignore_ascii_case("parquet"))
        {
            return Err(PlenoraError::Unsupported(
                "l'output deve avere estensione .parquet".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraError::Unsupported(
                "GeoParquet: un solo layer per dataset nella v1".to_owned(),
            ));
        }
        let layer = &plan.layers[0];
        let schema = layer.contract.schema.clone();
        let (geom_idx, geom_name, crs_meta) = geometry_field(&schema)?;
        // Schema di scrittura = utente + colonne bbox covering (spatial pruning).
        let mut aug_fields: Vec<Field> =
            schema.fields().iter().map(|f| f.as_ref().clone()).collect();
        aug_fields.extend(bbox_fields());
        let write_schema: SchemaRef =
            Arc::new(Schema::new_with_metadata(aug_fields, schema.metadata().clone()));
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let temp = tempfile::NamedTempFile::new_in(&parent)?;
        // Row group da 64k righe: statistiche min/max abbastanza granulari da
        // rendere efficace il row-group pruning in lettura (Fase 2C).
        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .set_max_row_group_row_count(Some(65_536))
            .build();
        let writer = ArrowWriter::try_new(temp.reopen()?, write_schema.clone(), Some(props))
            .map_err(|e| fmt_err(format!("writer: {e}")))?;
        Ok(Box::new(GeoParquetWriter {
            temp: Some(temp),
            writer: Some(writer),
            write_schema,
            path,
            durable: opts.durable,
            geom_idx,
            geom_name,
            crs_meta,
            geometry_types: BTreeSet::new(),
        }))
    }
}

struct GeoParquetDataset {
    path: PathBuf,
    out_schema: SchemaRef,
    has_bbox: bool,
    layers: Vec<LayerContract>,
}

impl OpenDatasetHandle for GeoParquetDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }

    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&self.path)?)
            .map_err(|e| fmt_err(format!("Parquet non valido: {e}")))?;

        // Projection pushdown (Fase 2C): se richiesto, leggi SOLO quelle colonne.
        // Con bbox covering, le colonne bbox interne sono SEMPRE proiettate via.
        let (builder, out_schema, layer) = match &request.projected_fields {
            None if self.has_bbox => {
                let idx: Vec<usize> = (0..self.out_schema.fields().len()).collect();
                let mask = ProjectionMask::roots(builder.parquet_schema(), idx);
                (
                    builder.with_projection(mask),
                    self.out_schema.clone(),
                    self.layers[0].clone(),
                )
            }
            None => (builder, self.out_schema.clone(), self.layers[0].clone()),
            Some(field_ids) => {
                let ncols = self.out_schema.fields().len();
                let mut idx: Vec<usize> = Vec::new();
                for fid in field_ids {
                    let i = fid.0 as usize;
                    if i >= ncols {
                        if request.projection_mode == ProjectionMode::Required {
                            return Err(PlenoraError::Unsupported(format!(
                                "projection Required: field id {} fuori range",
                                fid.0
                            )));
                        }
                        continue;
                    }
                    if !idx.contains(&i) {
                        idx.push(i);
                    }
                }
                idx.sort_unstable();
                // Schema proiettato: sottoinsieme in ordine originale (geometria già
                // ri-etichettata geoarrow.wkb se presente fra le colonne scelte).
                let fields: Vec<Field> = idx
                    .iter()
                    .map(|&i| self.out_schema.field(i).as_ref().clone())
                    .collect();
                let projected: SchemaRef = Arc::new(Schema::new_with_metadata(
                    fields,
                    self.out_schema.metadata().clone(),
                ));
                let mask = ProjectionMask::roots(builder.parquet_schema(), idx.iter().copied());
                let mut layer = self.layers[0].clone();
                layer.contract = DataContract {
                    schema: projected.clone(),
                    geometry: layer.contract.geometry.and_then(|g| {
                        projected.index_of(&g.name).ok().map(|i| GeometryColumnContract {
                            field_id: FieldId(i as u32),
                            ..g
                        })
                    }),
                };
                (builder.with_projection(mask), projected, layer)
            }
        };

        // Batch sizing adattivo (2C): onora batch_target.max_rows.
        let builder = builder.with_batch_size(request.batch_target.max_rows.max(1));
        // Row-group pruning (2C): salta i row group esclusi dalle statistiche
        // min/max (mai filtering riga-per-riga; over-return, mai under-return).
        let builder = apply_pruning(builder, &request.pruning_predicate);
        // Spatial pruning (2C): salta i row group il cui bbox non interseca l'hint.
        let builder =
            apply_spatial_pruning(builder, &request.spatial_pruning_hint, self.has_bbox);

        let reader = builder
            .build()
            .map_err(|e| fmt_err(format!("lettura: {e}")))?;
        Ok(Box::new(GeoParquetReader {
            reader,
            out_schema,
            layer,
        }))
    }
}

struct GeoParquetReader {
    reader: ParquetRecordBatchReader,
    out_schema: SchemaRef,
    layer: LayerContract,
}

impl LayerReader for GeoParquetReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        match self.reader.next() {
            None => Ok(None),
            Some(Err(e)) => Err(fmt_err(format!("batch: {e}"))),
            Some(Ok(batch)) => {
                // Ri-etichetta lo schema (geometria -> geoarrow.wkb) senza toccare
                // i buffer: pass-through delle colonne.
                let retagged =
                    RecordBatch::try_new(self.out_schema.clone(), batch.columns().to_vec())
                        .map_err(|e| fmt_err(format!("re-tag schema: {e}")))?;
                Ok(Some(retagged))
            }
        }
    }
}

struct GeoParquetWriter {
    temp: Option<tempfile::NamedTempFile>,
    writer: Option<ArrowWriter<File>>,
    write_schema: SchemaRef,
    path: PathBuf,
    durable: bool,
    geom_idx: usize,
    geom_name: String,
    crs_meta: Option<String>,
    geometry_types: BTreeSet<&'static str>,
}

impl FormatWriter for GeoParquetWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let geom = batch.column(self.geom_idx);
        accumulate_geometry_types(geom, &mut self.geometry_types);
        // Aggiunge le 4 colonne bbox covering per il pruning spaziale.
        let bbox_cols = match geom.as_any().downcast_ref::<BinaryArray>() {
            Some(g) => build_bbox_columns(g),
            None => bbox_fields()
                .iter()
                .map(|_| Arc::new(Float64Array::new_null(batch.num_rows())) as ArrayRef)
                .collect(),
        };
        let mut cols: Vec<ArrayRef> = batch.columns().to_vec();
        cols.extend(bbox_cols);
        let aug = RecordBatch::try_new(self.write_schema.clone(), cols)
            .map_err(|e| fmt_err(format!("augment bbox: {e}")))?;
        self.writer
            .as_mut()
            .expect("writer vivo")
            .write(&aug)
            .map_err(|e| fmt_err(format!("write: {e}")))
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        let mut writer = self.writer.take().expect("writer vivo");
        let geo = build_geo_metadata(
            &self.geom_name,
            &self.geometry_types,
            self.crs_meta.as_deref(),
        );
        writer.append_key_value_metadata(KeyValue::new("geo".to_owned(), geo));
        writer.close().map_err(|e| fmt_err(format!("close: {e}")))?;
        let temp = self.temp.take().expect("temp vivo");
        let (bytes, outcome) = publish_file_atomic(temp, &self.path, self.durable)?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            outcome,
        })
    }
}

// --- helpers ---------------------------------------------------------------

// --- row-group pruning (2C) ------------------------------------------------

#[derive(Clone, Copy)]
enum PruneOp {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
}

/// Predicato opaco "colonna OP valore" (OP: >, >=, <, <=, =/==).
fn parse_predicate(s: &str) -> Option<(String, PruneOp, f64)> {
    for (sym, op) in [
        (">=", PruneOp::Ge),
        ("<=", PruneOp::Le),
        ("==", PruneOp::Eq),
        (">", PruneOp::Gt),
        ("<", PruneOp::Lt),
        ("=", PruneOp::Eq),
    ] {
        if let Some((l, r)) = s.split_once(sym) {
            let col = l.trim().to_owned();
            if let Ok(val) = r.trim().parse::<f64>() {
                if !col.is_empty() {
                    return Some((col, op, val));
                }
            }
        }
    }
    None
}

fn stat_range(stats: &Statistics) -> Option<(f64, f64)> {
    match stats {
        Statistics::Int32(s) => Some((*s.min_opt()? as f64, *s.max_opt()? as f64)),
        Statistics::Int64(s) => Some((*s.min_opt()? as f64, *s.max_opt()? as f64)),
        Statistics::Float(s) => Some((*s.min_opt()? as f64, *s.max_opt()? as f64)),
        Statistics::Double(s) => Some((*s.min_opt()?, *s.max_opt()?)),
        _ => None,
    }
}

fn range_matches(min: f64, max: f64, op: PruneOp, v: f64) -> bool {
    match op {
        PruneOp::Gt => max > v,
        PruneOp::Ge => max >= v,
        PruneOp::Lt => min < v,
        PruneOp::Le => min <= v,
        PruneOp::Eq => min <= v && v <= max,
    }
}

/// Seleziona i row group che POSSONO soddisfare il predicato (over-return: se le
/// statistiche mancano o il predicato non è riconosciuto, tiene tutto).
fn apply_pruning(
    builder: ParquetRecordBatchReaderBuilder<File>,
    pred: &Option<PruningPredicate>,
) -> ParquetRecordBatchReaderBuilder<File> {
    let Some(PruningPredicate::Opaque(expr)) = pred else {
        return builder;
    };
    let Some((col, op, val)) = parse_predicate(expr) else {
        return builder;
    };
    let schema = builder.parquet_schema();
    let cidx = match (0..schema.num_columns()).find(|&i| schema.column(i).name() == col.as_str()) {
        Some(i) => i,
        None => return builder,
    };
    let md = builder.metadata();
    let mut keep = Vec::new();
    for rg in 0..md.num_row_groups() {
        let keep_it = match md.row_group(rg).column(cidx).statistics().and_then(stat_range) {
            Some((min, max)) => range_matches(min, max, op, val),
            None => true,
        };
        if keep_it {
            keep.push(rg);
        }
    }
    builder.with_row_groups(keep)
}

/// Spatial pruning: tiene i row group il cui estensione bbox interseca l'hint.
/// Over-return (stats mancanti → tiene); mai under-return.
fn apply_spatial_pruning(
    builder: ParquetRecordBatchReaderBuilder<File>,
    hint: &Option<Bbox>,
    has_bbox: bool,
) -> ParquetRecordBatchReaderBuilder<File> {
    let (Some(q), true) = (hint, has_bbox) else {
        return builder;
    };
    let schema = builder.parquet_schema();
    let leaf = |name: &str| (0..schema.num_columns()).find(|&i| schema.column(i).name() == name);
    let (Some(cminx), Some(cminy), Some(cmaxx), Some(cmaxy)) = (
        leaf("_bbox_minx"),
        leaf("_bbox_miny"),
        leaf("_bbox_maxx"),
        leaf("_bbox_maxy"),
    ) else {
        return builder;
    };
    let md = builder.metadata();
    let mut keep = Vec::new();
    for rg in 0..md.num_row_groups() {
        let g = md.row_group(rg);
        // Estensione del row group: min(minx),min(miny) .. max(maxx),max(maxy).
        let ext = (
            g.column(cminx).statistics().and_then(stat_range).map(|(a, _)| a),
            g.column(cminy).statistics().and_then(stat_range).map(|(a, _)| a),
            g.column(cmaxx).statistics().and_then(stat_range).map(|(_, b)| b),
            g.column(cmaxy).statistics().and_then(stat_range).map(|(_, b)| b),
        );
        let keep_it = match ext {
            (Some(minx), Some(miny), Some(maxx), Some(maxy)) => {
                // Interseca l'hint? (nessuna intersezione = fuori da un lato)
                !(maxx < q.minx || minx > q.maxx || maxy < q.miny || miny > q.maxy)
            }
            _ => true,
        };
        if keep_it {
            keep.push(rg);
        }
    }
    builder.with_row_groups(keep)
}

fn reject_zstd(builder: &ParquetRecordBatchReaderBuilder<File>) -> Result<()> {
    let pm = builder.metadata();
    for g in 0..pm.num_row_groups() {
        for c in 0..pm.row_group(g).num_columns() {
            if matches!(
                pm.row_group(g).column(c).compression(),
                Compression::ZSTD(_)
            ) {
                return Err(PlenoraError::Unsupported(
                    "compressione zstd non supportata (build puro-Rust): ricomprimere in snappy/gzip/brotli/lz4"
                        .to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn read_geo_meta(builder: &ParquetRecordBatchReaderBuilder<File>) -> Option<serde_json::Value> {
    let kv = builder.metadata().file_metadata().key_value_metadata()?;
    let raw = kv
        .iter()
        .find(|e| e.key == "geo")
        .and_then(|e| e.value.clone())?;
    serde_json::from_str(&raw).ok()
}

/// Nome colonna geometria + CRS risolto dai metadati `geo`.
fn resolve_geometry_and_crs(
    schema: &Schema,
    geo: Option<&serde_json::Value>,
) -> Result<(String, ResolvedCrs)> {
    let primary = geo
        .and_then(|g| g.get("primary_column"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| {
            ["geometry", "geom", "wkb"]
                .iter()
                .find(|n| schema.index_of(n).is_ok())
                .map(|s| s.to_string())
        })
        .ok_or_else(|| fmt_err("nessuna colonna geometria: non è GeoParquet"))?;
    let crs = crs_from(geo, &primary);
    Ok((primary, crs))
}

fn crs_from(geo: Option<&serde_json::Value>, primary: &str) -> ResolvedCrs {
    let crs = geo
        .and_then(|g| g.get("columns"))
        .and_then(|c| c.get(primary))
        .and_then(|c| c.get("crs"));
    match crs {
        None | Some(serde_json::Value::Null) => ResolvedCrs::wgs84(),
        Some(v) => {
            let id = v.get("id").and_then(|i| {
                let a = i.get("authority").and_then(|a| a.as_str())?;
                let code = i.get("code").map(|c| match c {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })?;
                Some(format!("{a}:{code}"))
            });
            ResolvedCrs {
                id: id.clone().or(Some("PROJJSON".to_owned())),
                kind: if matches!(id.as_deref(), Some("OGC:CRS84") | Some("EPSG:4326")) {
                    CrsKind::Geographic
                } else {
                    CrsKind::Unknown
                },
                definition: Some(v.to_string()),
            }
        }
    }
}

// --- bbox covering (spatial pruning, 2C) -----------------------------------

/// Colonne bbox interne (covering "plenora"): 4 f64 flat per row, con statistiche
/// min/max Parquet per row group → pruning spaziale.
const BBOX_COLS: [&str; 4] = ["_bbox_minx", "_bbox_miny", "_bbox_maxx", "_bbox_maxy"];

fn is_bbox_col(name: &str) -> bool {
    BBOX_COLS.contains(&name)
}

fn bbox_fields() -> Vec<Field> {
    BBOX_COLS
        .iter()
        .map(|n| Field::new(*n, DataType::Float64, true))
        .collect()
}

fn upd(bb: &mut [f64; 4], x: f64, y: f64) {
    if x < bb[0] {
        bb[0] = x;
    }
    if y < bb[1] {
        bb[1] = y;
    }
    if x > bb[2] {
        bb[2] = x;
    }
    if y > bb[3] {
        bb[3] = y;
    }
}

fn rd_u32(b: &[u8], off: &mut usize, le: bool) -> Option<u32> {
    let s = b.get(*off..*off + 4)?;
    *off += 4;
    let a = [s[0], s[1], s[2], s[3]];
    Some(if le {
        u32::from_le_bytes(a)
    } else {
        u32::from_be_bytes(a)
    })
}

fn rd_f64(b: &[u8], off: &mut usize, le: bool) -> Option<f64> {
    let s = b.get(*off..*off + 8)?;
    *off += 8;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Some(if le {
        f64::from_le_bytes(a)
    } else {
        f64::from_be_bytes(a)
    })
}

fn scan_wkb(b: &[u8], off: &mut usize, bb: &mut [f64; 4], depth: u32) -> Option<()> {
    if depth > 32 {
        return None;
    }
    let le = *b.get(*off)? == 1;
    *off += 1;
    let raw = rd_u32(b, off, le)?;
    match raw {
        1 => {
            let x = rd_f64(b, off, le)?;
            let y = rd_f64(b, off, le)?;
            upd(bb, x, y);
        }
        2 => {
            let n = rd_u32(b, off, le)?;
            for _ in 0..n {
                let x = rd_f64(b, off, le)?;
                let y = rd_f64(b, off, le)?;
                upd(bb, x, y);
            }
        }
        3 => {
            let rings = rd_u32(b, off, le)?;
            for _ in 0..rings {
                let npts = rd_u32(b, off, le)?;
                for _ in 0..npts {
                    let x = rd_f64(b, off, le)?;
                    let y = rd_f64(b, off, le)?;
                    upd(bb, x, y);
                }
            }
        }
        4..=7 => {
            let n = rd_u32(b, off, le)?;
            for _ in 0..n {
                scan_wkb(b, off, bb, depth + 1)?;
            }
        }
        _ => return None, // Z/M/EWKB o tipo ignoto → niente bbox (conservativo)
    }
    Some(())
}

/// Bounding box 2D da WKB senza costruire geometrie. `None` se non-2D o malformato
/// (robusto al fuzzing: nessun panic, nessun loop illimitato).
fn wkb_bbox(bytes: &[u8]) -> Option<[f64; 4]> {
    let mut bb = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut off = 0usize;
    scan_wkb(bytes, &mut off, &mut bb, 0)?;
    if bb.iter().all(|v| v.is_finite()) {
        Some(bb)
    } else {
        None
    }
}

/// Costruisce le 4 colonne bbox per un batch, dalla colonna geometria WKB.
fn build_bbox_columns(geom: &BinaryArray) -> Vec<ArrayRef> {
    let n = geom.len();
    let (mut minx, mut miny, mut maxx, mut maxy) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    for i in 0..n {
        let bb = if geom.is_null(i) {
            None
        } else {
            wkb_bbox(geom.value(i))
        };
        match bb {
            Some([a, b, c, d]) => {
                minx.push(Some(a));
                miny.push(Some(b));
                maxx.push(Some(c));
                maxy.push(Some(d));
            }
            None => {
                minx.push(None);
                miny.push(None);
                maxx.push(None);
                maxy.push(None);
            }
        }
    }
    vec![
        Arc::new(Float64Array::from(minx)),
        Arc::new(Float64Array::from(miny)),
        Arc::new(Float64Array::from(maxx)),
        Arc::new(Float64Array::from(maxy)),
    ]
}

/// Ricostruisce lo schema marcando la geometria come `geoarrow.wkb`+`crs` ed
/// ESCLUDENDO le colonne bbox covering (interne, non esposte al consumatore).
fn retag_schema(schema: &Schema, geom_name: &str, crs: &ResolvedCrs) -> SchemaRef {
    let fields: Vec<Field> = schema
        .fields()
        .iter()
        .filter(|f| !is_bbox_col(f.name()))
        .map(|f| {
            if f.name() == geom_name {
                let mut md = f.metadata().clone();
                md.insert(
                    ARROW_EXTENSION_NAME_KEY.to_owned(),
                    GEOARROW_WKB_EXTENSION.to_owned(),
                );
                if let Some(id) = &crs.id {
                    md.insert(GEO_CRS_KEY.to_owned(), id.clone());
                }
                f.as_ref().clone().with_metadata(md)
            } else {
                f.as_ref().clone()
            }
        })
        .collect();
    Arc::new(Schema::new_with_metadata(fields, schema.metadata().clone()))
}

/// Trova la colonna geometria (`geoarrow.wkb`) in uno schema in scrittura.
fn geometry_field(schema: &Schema) -> Result<(usize, String, Option<String>)> {
    for (i, f) in schema.fields().iter().enumerate() {
        if plenora_core::geometry::is_geometry_field(f) {
            let crs = f.metadata().get(GEO_CRS_KEY).cloned();
            return Ok((i, f.name().clone(), crs));
        }
    }
    Err(fmt_err(
        "nessuna colonna geometria geoarrow.wkb nel contratto",
    ))
}

fn wkb_type_name(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 5 {
        return None;
    }
    let le = bytes[0] == 1;
    let raw = if le {
        u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]])
    } else {
        u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]])
    };
    let base = if raw & 0xE000_0000 != 0 {
        raw & 0xFF
    } else {
        raw % 1000
    };
    Some(match base {
        1 => "Point",
        2 => "LineString",
        3 => "Polygon",
        4 => "MultiPoint",
        5 => "MultiLineString",
        6 => "MultiPolygon",
        7 => "GeometryCollection",
        _ => return None,
    })
}

fn accumulate_geometry_types(col: &dyn Array, out: &mut BTreeSet<&'static str>) {
    if let Some(a) = col.as_any().downcast_ref::<BinaryArray>() {
        for i in 0..a.len() {
            if !a.is_null(i) {
                if let Some(t) = wkb_type_name(a.value(i)) {
                    out.insert(t);
                }
            }
        }
    } else if let Some(a) = col.as_any().downcast_ref::<LargeBinaryArray>() {
        for i in 0..a.len() {
            if !a.is_null(i) {
                if let Some(t) = wkb_type_name(a.value(i)) {
                    out.insert(t);
                }
            }
        }
    }
}

fn build_geo_metadata(
    geom_name: &str,
    types: &BTreeSet<&'static str>,
    crs: Option<&str>,
) -> String {
    let mut column = serde_json::json!({
        "encoding": "WKB",
        "geometry_types": types.iter().collect::<Vec<_>>(),
    });
    // crs "AUTH:CODE" -> {"id":{authority,code}}, altrimenti null.
    if let Some(id) = crs {
        if let Some((auth, code)) = id.split_once(':') {
            let code_val: serde_json::Value = code
                .parse::<i64>()
                .map(serde_json::Value::from)
                .unwrap_or_else(|_| serde_json::Value::from(code));
            column["crs"] = serde_json::json!({"id": {"authority": auth, "code": code_val}});
        }
    }
    let mut columns = HashMap::new();
    columns.insert(geom_name.to_owned(), column);
    serde_json::json!({
        "version": "1.0.0",
        "primary_column": geom_name,
        "columns": columns,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::Int64Array;
    use arrow_schema::DataType;
    use geo_types::{Geometry, Point};
    use plenora_core::wkb::to_wkb;
    use plenora_io_core::request::BatchTarget;
    use plenora_io_core::WriteLayer;

    fn geometry_field_meta(crs: &str) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert(
            ARROW_EXTENSION_NAME_KEY.to_owned(),
            GEOARROW_WKB_EXTENSION.to_owned(),
        );
        m.insert(GEO_CRS_KEY.to_owned(), crs.to_owned());
        m
    }

    #[test]
    fn round_trip_file_recordbatch_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.parquet");

        // RecordBatch con colonna geometria geoarrow.wkb + una colonna int.
        let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(12.5, 45.9))).unwrap();
        let geom = BinaryArray::from(vec![Some(wkb.as_slice()), Some(wkb.as_slice())]);
        let ids = Int64Array::from(vec![1i64, 2]);
        let schema = Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, true)
                .with_metadata(geometry_field_meta("EPSG:4326")),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(geom), Arc::new(ids)]).unwrap();

        // create -> write -> finish
        let driver = GeoParquetDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema: schema.clone(),
                    geometry: None,
                },
            }],
        };
        let mut writer = driver
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.write(&batch).unwrap();
        let published = writer.finish().unwrap();
        assert!(published.bytes > 0);

        // open -> read back
        let ds = driver
            .open(Source::Path(path.clone()), &ReadOptions::default())
            .unwrap();
        assert_eq!(ds.layers().len(), 1);
        let layer = &ds.layers()[0];
        let geom_c = layer.contract.geometry.as_ref().unwrap();
        assert_eq!(geom_c.name, "geometry");
        assert_eq!(geom_c.crs.id.as_deref(), Some("EPSG:4326"));

        let mut reader = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget::default(),
            })
            .unwrap();
        let out = reader.next_batch().unwrap().unwrap();
        assert_eq!(out.num_rows(), 2);
        // La geometria è marcata geoarrow.wkb nello schema effettivo.
        let field = out.schema().field_with_name("geometry").unwrap().clone();
        assert!(plenora_core::geometry::is_geometry_field(&field));
        // I byte WKB sono pass-through identici.
        let col = out
            .column_by_name("geometry")
            .unwrap()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(col.value(0), wkb.as_slice());
        assert!(reader.next_batch().unwrap().is_none());
    }

    /// Conformità alla spec GeoParquet 1.0.0, verificata riaprendo il file
    /// GREZZO col crate `parquet` (indipendente dal nostro reader): metadato
    /// `geo` file-level + colonna geometria fisicamente BYTE_ARRAY (WKB).
    #[test]
    fn geoparquet_spec_conformance() {
        use parquet::basic::Type as PhysicalType;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conf.parquet");

        let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(9.19, 45.46))).unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, true)
                .with_metadata(geometry_field_meta("EPSG:4326")),
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

        let driver = GeoParquetDriver;
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
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        // Riapri il file grezzo col crate parquet.
        let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&path).unwrap()).unwrap();

        // 1) metadato file-level "geo" conforme.
        let kv = builder
            .metadata()
            .file_metadata()
            .key_value_metadata()
            .expect("key_value_metadata assenti");
        let geo_raw = kv
            .iter()
            .find(|e| e.key == "geo")
            .and_then(|e| e.value.clone())
            .expect("metadato 'geo' assente (non è GeoParquet)");
        let geo: serde_json::Value = serde_json::from_str(&geo_raw).unwrap();

        assert_eq!(geo["version"].as_str(), Some("1.0.0"));
        assert_eq!(geo["primary_column"].as_str(), Some("geometry"));
        let col = &geo["columns"]["geometry"];
        assert_eq!(col["encoding"].as_str(), Some("WKB"));
        let types: Vec<&str> = col["geometry_types"]
            .as_array()
            .expect("geometry_types deve essere un array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(
            types.contains(&"Point"),
            "geometry_types deve contenere 'Point', era {types:?}"
        );
        assert_eq!(col["crs"]["id"]["authority"].as_str(), Some("EPSG"));
        assert_eq!(col["crs"]["id"]["code"].as_i64(), Some(4326));

        // 2) la colonna geometria è fisicamente BYTE_ARRAY (WKB) nel Parquet.
        let pschema = builder.metadata().file_metadata().schema_descr();
        let geom_col = (0..pschema.num_columns())
            .map(|i| pschema.column(i))
            .find(|c| c.name() == "geometry")
            .expect("colonna 'geometry' assente nello schema Parquet");
        assert_eq!(geom_col.physical_type(), PhysicalType::BYTE_ARRAY);
    }

    #[test]
    fn projection_pushdown_reads_only_requested() {
        use plenora_core::contract::FieldId;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proj.parquet");
        let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, true)
                .with_metadata(geometry_field_meta("EPSG:4326")),
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(Int64Array::from(vec![7i64])),
                Arc::new(arrow_array::StringArray::from(vec!["x"])),
            ],
        )
        .unwrap();

        let driver = GeoParquetDriver;
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
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        // Proietta SOLO la colonna "id" (indice 1) in modalità Required.
        let ds = driver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
        let mut reader = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: Some(vec![FieldId(1)]),
                projection_mode: ProjectionMode::Required,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget::default(),
            })
            .unwrap();
        // Il contratto del reader riflette la projection (1 colonna, niente geometria).
        assert_eq!(reader.contract().contract.schema.fields().len(), 1);
        assert!(reader.contract().contract.geometry.is_none());
        let out = reader.next_batch().unwrap().unwrap();
        assert_eq!(out.num_columns(), 1);
        assert_eq!(out.schema().field(0).name(), "id");
        assert_eq!(
            out.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .value(0),
            7
        );
    }

    #[test]
    fn row_group_pruning_skips_blocks() {
        use plenora_io_core::request::PruningPredicate;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prune.parquet");
        let wkb: Vec<u8> = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
        let n = 200_000usize; // > 3 row group (65536/row group)
        let schema = Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, true)
                .with_metadata(geometry_field_meta("EPSG:4326")),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(
                    (0..n).map(|_| Some(wkb.as_slice())).collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from((0..n as i64).collect::<Vec<_>>())),
            ],
        )
        .unwrap();

        let driver = GeoParquetDriver;
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
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        // Pruning "id > 150000": salta i row group con max(id) <= 150000.
        let ds = driver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
        let mut reader = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: Some(PruningPredicate::Opaque("id > 150000".to_owned())),
                spatial_pruning_hint: None,
                batch_target: BatchTarget::default(),
            })
            .unwrap();
        let mut total = 0;
        while let Some(b) = reader.next_batch().unwrap() {
            total += b.num_rows();
        }
        // Over-return: legge solo i row group che POSSONO contenere id>150000
        // (meno di tutte le 200k righe, ma tutte le righe matchanti sono incluse).
        assert!(total < n, "il pruning deve saltare row group, letti {total}");
        assert!(total >= n - 150_000, "under-return vietato, letti {total}");
    }

    #[test]
    fn spatial_pruning_skips_blocks() {
        use plenora_io_core::request::Bbox;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sp.parquet");
        let n = 200_000usize;
        // Punti con x crescente (0..200), y=45 → row group con estensione x diversa.
        let wkb: Vec<Vec<u8>> = (0..n)
            .map(|i| to_wkb(&Geometry::Point(Point::new(i as f64 * 0.001, 45.0))).unwrap())
            .collect();
        let schema = Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, true)
                .with_metadata(geometry_field_meta("EPSG:4326")),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(
                    wkb.iter().map(|w| Some(w.as_slice())).collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from((0..n as i64).collect::<Vec<_>>())),
            ],
        )
        .unwrap();

        let driver = GeoParquetDriver;
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
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        let ds = driver
            .open(Source::Path(path), &ReadOptions::default())
            .unwrap();
        // Il bbox covering NON è esposto: il contratto ha solo geometry + id.
        assert_eq!(ds.layers()[0].contract.schema.fields().len(), 2);

        // Hint spaziale x in [190,210]: interseca solo gli ultimi row group.
        let mut reader = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: Some(Bbox {
                    minx: 190.0,
                    miny: 40.0,
                    maxx: 210.0,
                    maxy: 50.0,
                }),
                batch_target: BatchTarget::default(),
            })
            .unwrap();
        let mut total = 0;
        while let Some(b) = reader.next_batch().unwrap() {
            // Il batch NON contiene le colonne bbox interne.
            assert_eq!(b.num_columns(), 2);
            total += b.num_rows();
        }
        // Pruning: legge meno di tutto ma include tutte le ~10000 righe con x in [190,200].
        assert!(total < n, "spatial pruning deve saltare row group, letti {total}");
        assert!(total >= 10_000, "under-return vietato, letti {total}");
    }
}
