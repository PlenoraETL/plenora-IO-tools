//! plenora-bench — harness di baseline prestazionale. NON ottimizza: misura
//! throughput, picco RSS, allocazioni e decode/encode WKB per driver, e archivia
//! una baseline. Metriche di `Prestazioni.md` §7 (`bytes_copied` e metriche di
//! coda: n/a in v1).
//!
//! Scala grande (10M righe) resa ONESTA:
//! - Generazione a **chunk** (64k righe): l'harness non tiene mai l'intero
//!   dataset in RAM, né in lettura né in scrittura.
//! - Fixture di lettura preparate in un **subprocesso separato** con writer
//!   streaming (testo per geojson/csv, driver per geoparquet/gpkg che streamano):
//!   così il subprocesso di `read` fa SOLO lettura e il suo picco RSS è la RAM
//!   vera del driver, non quella dell'input.
//! - Geometrie da un **pool** di WKB precalcolato (1024 blob): la generazione
//!   non chiama `to_wkb`, quindi i contatori decode/encode e le allocazioni in
//!   scrittura riflettono il DRIVER, non la generazione.
//! - Ogni benchmark gira in subprocesso con deadline: un OOM/hang (atteso sui
//!   driver materializzanti a 10M) uccide solo quel figlio e viene registrato
//!   come `failed` — è il finding, non un crash dell'intera baseline.
//!
//! Nota: usa `unsafe` (allocatore contatore + getrusage). Strumento di misura,
//! non codice di produzione.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arrow_array::{Array, BinaryArray, Float64Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};

use driver_common::geometry_field;
use plenora_io_core::driver::{FormatDriver, ReadOptions, Sink, Source, WriteOptions};
use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadRequest, ReadScope};
use plenora_io_core::{WriteLayer, WritePlan};
use plenora_io_model::contract::{
    DataContract, FieldId, GeometryColumnContract, GeometryType, LayerId,
};
use plenora_io_model::crs::{CrsKind, ResolvedCrs};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{decode_wkb, inspect_wkb, to_wkb};
use plenora_io_model::CancellationToken;

const CHUNK: usize = 65_536;
const POOL: usize = 1024;

// --- allocatore contatore --------------------------------------------------

static ALLOCATED: AtomicU64 = AtomicU64::new(0);
static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static CURRENT: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);

struct Counting;

// SAFETY: delega a System aggiornando solo contatori atomici.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            ALLOCATED.fetch_add(l.size() as u64, Ordering::Relaxed);
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            let cur = CURRENT.fetch_add(l.size() as u64, Ordering::Relaxed) + l.size() as u64;
            PEAK.fetch_max(cur, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        CURRENT.fetch_sub(l.size() as u64, Ordering::Relaxed);
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn reset_alloc() {
    ALLOCATED.store(0, Ordering::Relaxed);
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    PEAK.store(CURRENT.load(Ordering::Relaxed), Ordering::Relaxed);
}

// --- metriche di sistema ---------------------------------------------------

#[cfg(unix)]
fn cpu_ms() -> f64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe {
        libc::getrusage(libc::RUSAGE_SELF, &raw mut u);
    }
    // Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e
    // violerebbe il determinismo bit-esatto (ADR-0001). I campi di `timeval`
    // sono secondi/microsecondi di CPU: ampiamente sotto 2^53, cast esatto.
    #[allow(
        clippy::suboptimal_flops,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap
    )]
    let s = |t: libc::timeval| t.tv_sec as f64 * 1000.0 + t.tv_usec as f64 / 1000.0;
    s(u.ru_utime) + s(u.ru_stime)
}

#[cfg(not(unix))]
fn cpu_ms() -> f64 {
    // `getrusage` non è disponibile: la metrica resta esplicitamente n/a.
    0.0
}

fn peak_rss_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
        })
        .map_or(0, |kb| kb * 1024)
}

// --- generazione a chunk (pool WKB, niente encode nella generazione) -------

fn bench_crs(id: &str) -> &'static str {
    if matches!(id, "geojson" | "kml") {
        driver_common::OGC_CRS84
    } else {
        "EPSG:4326"
    }
}

fn bench_schema(id: &str) -> SchemaRef {
    Arc::new(Schema::new(vec![
        geometry_field("geometry", bench_crs(id)),
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("val", DataType::Float64, false),
    ]))
}

fn bench_contract(id: &str) -> DataContract {
    let crs = if matches!(id, "geojson" | "kml") {
        ResolvedCrs::wgs84()
    } else {
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None)
    };
    let mut geometry = GeometryColumnContract::wkb_xy(FieldId(0), "geometry", crs, true);
    geometry.set_exact_geometry_types(vec![if use_polygon() {
        GeometryType::Polygon
    } else {
        GeometryType::Point
    }]);
    DataContract {
        schema: bench_schema(id),
        geometry: Some(geometry),
    }
}

// Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e violerebbe il
// determinismo bit-esatto delle fixture di baseline (ADR-0001). I resti sono
// < 1000: la conversione a f64 è esatta.
#[allow(clippy::suboptimal_flops, clippy::cast_precision_loss)]
fn coord(k: usize) -> (f64, f64) {
    (
        6.0 + (k % 1000) as f64 * 0.001,
        45.0 + (k % 777) as f64 * 0.001,
    )
}

/// Se `PLENORA_BENCH_GEOM=polygon`, i benchmark usano poligoni (un quadratino)
/// invece di punti: così le metriche riflettono il costo del percorso geometria
/// (che con soli Point è quasi nullo).
fn use_polygon() -> bool {
    std::env::var("PLENORA_BENCH_GEOM")
        .map(|v| v == "polygon")
        .unwrap_or(false)
}

/// Anello quadrato (chiuso) attorno a `coord(k)`.
// `[x, y]` qui è un vertice dell'anello, non la conversione di una tupla:
// `tuple_array_conversions` è un falso positivo.
#[allow(clippy::tuple_array_conversions)]
fn poly_ring(k: usize) -> Vec<[f64; 2]> {
    let (x, y) = coord(k);
    let d = 0.0005;
    vec![[x, y], [x + d, y], [x + d, y + d], [x, y + d], [x, y]]
}

fn wkb_pool() -> Vec<Vec<u8>> {
    let poly = use_polygon();
    (0..POOL)
        .map(|k| {
            let g = if poly {
                let ring: Vec<geo_types::Coord<f64>> = poly_ring(k)
                    .into_iter()
                    .map(|p| geo_types::Coord { x: p[0], y: p[1] })
                    .collect();
                geo_types::Geometry::Polygon(geo_types::Polygon::new(
                    geo_types::LineString(ring),
                    vec![],
                ))
            } else {
                let (x, y) = coord(k);
                geo_types::Geometry::Point(geo_types::Point::new(x, y))
            };
            to_wkb(&g).unwrap()
        })
        .collect()
}

fn name_pool() -> Vec<String> {
    (0..POOL).map(|k| format!("f{k}")).collect()
}

fn gen_chunk(
    id: &str,
    pool: &[Vec<u8>],
    names: &[String],
    start: usize,
    count: usize,
) -> RecordBatch {
    let geom = BinaryArray::from(
        (0..count)
            .map(|j| Some(pool[(start + j) % POOL].as_slice()))
            .collect::<Vec<_>>(),
    );
    // Indici di riga della fixture (≤ 10M): i cast a i64 e f64 sono esatti.
    #[allow(clippy::cast_possible_wrap)]
    let ids = Int64Array::from((0..count).map(|j| (start + j) as i64).collect::<Vec<_>>());
    let nm = StringArray::from(
        (0..count)
            .map(|j| names[(start + j) % POOL].as_str())
            .collect::<Vec<_>>(),
    );
    #[allow(clippy::cast_precision_loss)]
    let vals = Float64Array::from(
        (0..count)
            .map(|j| (start + j) as f64 * 1.5)
            .collect::<Vec<_>>(),
    );
    RecordBatch::try_new(
        bench_schema(id),
        vec![Arc::new(geom), Arc::new(ids), Arc::new(nm), Arc::new(vals)],
    )
    .unwrap()
}

// --- driver / fixture ------------------------------------------------------

fn driver_by_id(id: &str) -> Box<dyn FormatDriver> {
    match id {
        "geoparquet" => Box::new(driver_geoparquet::GeoParquetDriver),
        "geojson" => Box::new(driver_geojson::GeoJsonDriver),
        "csv" => Box::new(driver_csv::CsvDriver),
        "gpkg" => Box::new(driver_gpkg::GpkgDriver),
        "kml" => Box::new(driver_kml::KmlDriver),
        "dxf" => Box::new(driver_dxf::DxfDriver),
        "shp" => Box::new(driver_shp::ShpDriver),
        "xlsx" => Box::new(driver_xls::XlsDriver),
        other => panic!("driver sconosciuto: {other}"),
    }
}

fn ext(id: &str) -> &'static str {
    match id {
        "geoparquet" => "parquet",
        "geojson" => "geojson",
        "csv" => "csv",
        "gpkg" => "gpkg",
        "kml" => "kml",
        "dxf" => "dxf",
        "shp" => "shp",
        "xlsx" => "xlsx",
        _ => "bin",
    }
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(std::env::var("PLENORA_BENCH_FIXDIR").unwrap_or_else(|_| "bench-fix".to_owned()))
}

fn fixture_path(id: &str) -> PathBuf {
    fixture_dir().join(format!("{id}.{}", ext(id)))
}

fn remove_dataset(id: &str, path: &Path) {
    if id == "shp" {
        for extension in ["dbf", "prj", "shp", "shx"] {
            std::fs::remove_file(path.with_extension(extension)).ok();
        }
    } else {
        std::fs::remove_file(path).ok();
    }
}

fn read_opts(id: &str) -> ReadOptions {
    let mut o = ReadOptions::default();
    if matches!(id, "csv" | "xlsx") {
        o.assume_crs = Some("EPSG:4326".to_owned());
        o.format_options
            .insert("wkt_column".to_owned(), "geometry".to_owned());
    }
    o
}

/// Scrittura a chunk via driver (streaming se il driver lo è; usato per i
/// fixture geoparquet/gpkg e per il benchmark di `write`). Ritorna
/// (`n_batch`, `max_batch_bytes`).
fn feed_write(
    id: &str,
    path: &Path,
    total: usize,
    pool: &[Vec<u8>],
    names: &[String],
) -> (usize, usize) {
    remove_dataset(id, path);
    let driver = driver_by_id(id);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "bench".to_owned(),
            contract: bench_contract(id),
        }],
    };
    let mut w = driver
        .create(Sink::Path(path.to_owned()), &plan, &WriteOptions::default())
        .unwrap();
    let mut start = 0;
    let mut batches = 0;
    let mut max_bb = 0;
    while start < total {
        let c = (total - start).min(CHUNK);
        let batch = gen_chunk(id, pool, names, start, c);
        max_bb = max_bb.max(batch.get_array_memory_size());
        w.write(&batch).unwrap();
        batches += 1;
        start += c;
    }
    w.finish().unwrap();
    (batches, max_bb)
}

/// Fixture geojson streaming (testo, mai bufferizzato) — evita l'OOM del driver
/// geojson (che bufferizza) durante la *preparazione*.
fn write_geojson_fixture(path: &Path, total: usize) {
    let f = std::fs::File::create(path).unwrap();
    let mut w = BufWriter::new(f);
    w.write_all(b"{\"type\":\"FeatureCollection\",\"features\":[")
        .unwrap();
    let poly = use_polygon();
    for i in 0..total {
        if i > 0 {
            w.write_all(b",").unwrap();
        }
        let geom = if poly {
            let r = poly_ring(i);
            format!(
                "{{\"type\":\"Polygon\",\"coordinates\":[[[{},{}],[{},{}],[{},{}],[{},{}],[{},{}]]]}}",
                r[0][0], r[0][1], r[1][0], r[1][1], r[2][0], r[2][1], r[3][0], r[3][1], r[4][0], r[4][1]
            )
        } else {
            let (x, y) = coord(i);
            format!("{{\"type\":\"Point\",\"coordinates\":[{x},{y}]}}")
        };
        // `i` < 10M < 2^53: la conversione a f64 è esatta.
        #[allow(clippy::cast_precision_loss)]
        let val = i as f64 * 1.5;
        write!(
            w,
            "{{\"type\":\"Feature\",\"geometry\":{geom},\"properties\":{{\"id\":{i},\"name\":\"f{}\",\"val\":{val}}}}}",
            i % POOL
        )
        .unwrap();
    }
    w.write_all(b"]}").unwrap();
    w.flush().unwrap();
}

/// Fixture csv streaming (WKT nella colonna `geometry`).
fn write_csv_fixture(path: &Path, total: usize) {
    let f = std::fs::File::create(path).unwrap();
    let mut w = BufWriter::new(f);
    w.write_all(b"id,name,val,geometry\n").unwrap();
    for i in 0..total {
        let (x, y) = coord(i);
        // `i` < 10M < 2^53: la conversione a f64 è esatta.
        #[allow(clippy::cast_precision_loss)]
        let val = i as f64 * 1.5;
        writeln!(w, "{i},f{},{val},\"POINT ({x} {y})\"", i % POOL).unwrap();
    }
    w.flush().unwrap();
}

struct ReadStats {
    rows: usize,
    geometries: usize,
    batches: usize,
    max_batch_bytes: usize,
    total_batch_bytes: usize,
}

fn read_drain(
    id: &str,
    path: &Path,
    projected: Option<Vec<plenora_io_model::contract::FieldId>>,
    pruning: Option<String>,
) -> ReadStats {
    let has_proj = projected.is_some();
    let driver = driver_by_id(id);
    let ds = driver
        .open(Source::Path(path.to_owned()), read_opts(id))
        .unwrap();
    // Indice geometria dallo schema pieno (solo se NON stiamo proiettando).
    let geom_idx = if has_proj {
        None
    } else {
        Some(
            ds.layers()[0]
                .contract
                .geometry
                .as_ref()
                .and_then(|g| ds.layers()[0].contract.schema.index_of(&g.name).ok())
                .unwrap_or(0),
        )
    };
    let mut reader = ds
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: projected,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: pruning.map(plenora_io_core::request::PruningPredicate::Opaque),
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        })
        .unwrap();
    let mut st = ReadStats {
        rows: 0,
        geometries: 0,
        batches: 0,
        max_batch_bytes: 0,
        total_batch_bytes: 0,
    };
    while let Some(batch) = reader.next_batch().unwrap() {
        st.rows += batch.num_rows();
        if let Some(gi) = geom_idx {
            st.geometries += batch.num_rows() - batch.column(gi).null_count();
        }
        let b = batch.get_array_memory_size();
        st.total_batch_bytes += b;
        st.max_batch_bytes = st.max_batch_bytes.max(b);
        st.batches += 1;
    }
    st
}

// --- esecuzione di un benchmark (subprocesso figlio) ----------------------

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn dataset_len(id: &str, path: &Path) -> u64 {
    if id == "shp" {
        ["dbf", "prj", "shp", "shx"]
            .into_iter()
            .map(|extension| file_len(&path.with_extension(extension)))
            .sum()
    } else {
        file_len(path)
    }
}

// Un solo flusso per operazione (prepare/read/write): reset dei contatori,
// esecuzione e raccolta metriche devono restare adiacenti perché l'ordine
// determina cosa finisce nella baseline.
#[allow(clippy::too_many_lines)]
fn run_one(id: &str, op: &str, rows: usize) -> serde_json::Value {
    // Pool costruiti PRIMA del reset: i loro 1024 encode non contano.
    let pool = wkb_pool();
    let names = name_pool();

    if op == "prepare" {
        let path = fixture_path(id);
        std::fs::create_dir_all(fixture_dir()).ok();
        match id {
            "geojson" => write_geojson_fixture(&path, rows),
            "csv" => write_csv_fixture(&path, rows),
            _ => {
                feed_write(id, &path, rows, &pool, &names);
            }
        }
        return serde_json::json!({"prepared": id, "bytes": dataset_len(id, &path)});
    }

    plenora_io_model::metrics::reset();
    reset_alloc();
    let cpu0 = cpu_ms();
    let t0 = Instant::now();

    let rows_done;
    let geoms;
    let batches;
    let max_bb;
    let total_bb;
    let io_bytes;
    if op == "wkb_decode" || op == "wkb_inspect" {
        let limits = WkbLimits::default();
        let mut logical_bytes = 0u64;
        for index in 0..rows {
            let bytes = &pool[index % pool.len()];
            logical_bytes = logical_bytes.saturating_add(bytes.len() as u64);
            if op == "wkb_decode" {
                std::hint::black_box(decode_wkb(bytes, &limits).unwrap());
            } else {
                std::hint::black_box(inspect_wkb(bytes, &limits).unwrap());
            }
        }
        rows_done = rows;
        geoms = rows;
        batches = 0;
        max_bb = 0;
        total_bb = 0;
        io_bytes = logical_bytes;
    } else if op == "read" || op == "read_proj" || op == "read_pruned" {
        let path = fixture_path(id);
        // read_proj: proietta solo id (1) + val (3), saltando geometria(0) e name(2).
        let proj = if op == "read_proj" {
            Some(vec![
                plenora_io_model::contract::FieldId(1),
                plenora_io_model::contract::FieldId(3),
            ])
        } else {
            None
        };
        // read_pruned: salta i row group con id <= 90% delle righe.
        let pruning = if op == "read_pruned" {
            Some(format!("id > {}", rows * 9 / 10))
        } else {
            None
        };
        let s = read_drain(id, &path, proj, pruning);
        rows_done = s.rows;
        geoms = s.geometries;
        batches = s.batches;
        max_bb = s.max_batch_bytes;
        total_bb = s.total_batch_bytes;
        io_bytes = dataset_len(id, &path);
    } else {
        let out = fixture_dir().join(format!("{id}-out.{}", ext(id)));
        std::fs::create_dir_all(fixture_dir()).ok();
        let (nb, mbb) = feed_write(id, &out, rows, &pool, &names);
        io_bytes = dataset_len(id, &out);
        remove_dataset(id, &out);
        rows_done = rows;
        geoms = rows;
        batches = nb;
        max_bb = mbb;
        total_bb = mbb;
    }

    let wall_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let cpu = cpu_ms() - cpu0;
    let (wkb_decode, wkb_encode) = plenora_io_model::metrics::snapshot();
    let bytes_alloc = ALLOCATED.load(Ordering::Relaxed);
    let alloc_count = ALLOC_COUNT.load(Ordering::Relaxed);
    let peak_heap = PEAK.load(Ordering::Relaxed);
    let peak_rss = peak_rss_bytes();
    // Byte di I/O e righe della baseline restano sotto 2^53: i cast a f64 sono
    // esatti e non alterano le metriche riportate.
    #[allow(clippy::cast_precision_loss)]
    let mb = io_bytes as f64 / 1_048_576.0;
    #[allow(clippy::cast_precision_loss)]
    let rows_done_f64 = rows_done as f64;
    let secs = wall_ms / 1000.0;

    serde_json::json!({
        "driver": id,
        "op": op,
        "status": "ok",
        "rows": rows_done,
        "geometries": geoms,
        "coordinates": geoms,
        "batches": batches,
        "wall_ms": (wall_ms * 100.0).round() / 100.0,
        "cpu_ms": (cpu * 100.0).round() / 100.0,
        "rows_per_s": if secs > 0.0 { (rows_done_f64 / secs).round() } else { 0.0 },
        "mb_per_s": if secs > 0.0 { (mb / secs * 100.0).round() / 100.0 } else { 0.0 },
        "peak_rss_bytes": peak_rss,
        "peak_heap_bytes": peak_heap,
        "bytes_allocated": bytes_alloc,
        "allocation_count": alloc_count,
        "wkb_decode_count": wkb_decode,
        "wkb_encode_count": wkb_encode,
        "avg_batch_bytes": if batches > 0 { total_bb / batches } else { 0 },
        "max_batch_bytes": max_bb,
        "io_bytes": io_bytes,
        "bytes_copied": serde_json::Value::Null,
    })
}

// --- orchestratore ---------------------------------------------------------

fn run_child(exe: &Path, args: &[&str], deadline: Duration) -> serde_json::Value {
    let child = std::process::Command::new(exe)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return serde_json::json!({"status":"failed","reason":format!("spawn: {e}")}),
    };
    let start = Instant::now();
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) => {
                if start.elapsed() > deadline {
                    child.kill().ok();
                    timed_out = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => return serde_json::json!({"status":"failed","reason":format!("wait: {e}")}),
    };
    if timed_out {
        return serde_json::json!({"status":"failed","reason":"timeout (deadline superata)"});
    }
    if out.status.success() {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
            return v;
        }
    }
    let code = out.status.code();
    let tail: String = String::from_utf8_lossy(&out.stderr)
        .lines()
        .last()
        .unwrap_or("")
        .to_owned();
    serde_json::json!({
        "status": "failed",
        "reason": format!("exit={:?} (probabile OOM se ucciso da segnale) {}", code, tail),
    })
}

// `f64::midpoint` NON è bit-identico a `(a + b) / 2.0`: cambierebbe la mediana
// pubblicata nelle baseline storiche. La media aritmetica esplicita resta il
// contratto numerico di questo harness (ADR-0001, determinismo bit-esatto).
#[allow(clippy::manual_midpoint)]
fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

type BenchmarkKey = (String, String);
type BenchmarkMedian = (f64, f64);
type BenchmarkMedians = BTreeMap<BenchmarkKey, BenchmarkMedian>;

fn benchmark_medians(
    document: &serde_json::Value,
) -> std::result::Result<BenchmarkMedians, String> {
    let benchmarks = document["benchmarks"]
        .as_array()
        .ok_or_else(|| "campo benchmarks assente".to_owned())?;
    let mut samples: BTreeMap<(String, String), (Vec<f64>, Vec<f64>)> = BTreeMap::new();
    for benchmark in benchmarks {
        if benchmark["status"].as_str() != Some("ok") {
            continue;
        }
        let driver = benchmark["driver"]
            .as_str()
            .ok_or_else(|| "driver assente".to_owned())?;
        let op = benchmark["op"]
            .as_str()
            .ok_or_else(|| "op assente".to_owned())?;
        let rows_per_s = benchmark["rows_per_s"]
            .as_f64()
            .ok_or_else(|| format!("{driver}/{op}: rows_per_s assente"))?;
        let peak_rss = benchmark["peak_rss_bytes"]
            .as_f64()
            .ok_or_else(|| format!("{driver}/{op}: peak_rss_bytes assente"))?;
        let entry = samples
            .entry((driver.to_owned(), op.to_owned()))
            .or_default();
        entry.0.push(rows_per_s);
        entry.1.push(peak_rss);
    }
    samples
        .into_iter()
        .map(|(key, (mut throughput, mut rss))| {
            if throughput.is_empty() || rss.is_empty() {
                return Err(format!("{}/{}: nessun campione valido", key.0, key.1));
            }
            Ok((key, (median(&mut throughput), median(&mut rss))))
        })
        .collect()
}

fn percent_change(before: f64, after: f64) -> f64 {
    if before == 0.0 {
        if after == 0.0 {
            0.0
        } else {
            f64::INFINITY
        }
    } else {
        (after - before) / before * 100.0
    }
}

fn compare_baselines(before_path: &Path, after_path: &Path) -> std::result::Result<(), String> {
    let load = |path: &Path| {
        let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|error| format!("{}: {error}", path.display()))
    };
    let before = load(before_path)?;
    let after = load(after_path)?;
    for document in [&before, &after] {
        if document["contract"].as_str() != Some("plenora-io-baseline-v1") {
            return Err("contratto baseline non riconosciuto".to_owned());
        }
    }
    for field in ["rows_per_benchmark", "geometry"] {
        if before[field] != after[field] {
            return Err(format!(
                "baseline non comparabili: {field} {:?} != {:?}",
                before[field], after[field]
            ));
        }
    }
    let max_throughput_regression = std::env::var("PLENORA_BENCH_MAX_THROUGHPUT_REGRESSION_PCT")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(5.0);
    let max_rss_regression = std::env::var("PLENORA_BENCH_MAX_RSS_REGRESSION_PCT")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(5.0);
    let before_medians = benchmark_medians(&before)?;
    let after_medians = benchmark_medians(&after)?;
    let mut failures = Vec::new();
    println!("driver/op         throughput delta    peak RSS delta    esito");
    for (key, (before_rate, before_rss)) in &before_medians {
        let Some((after_rate, after_rss)) = after_medians.get(key) else {
            failures.push(format!("{}/{}: risultato post mancante", key.0, key.1));
            continue;
        };
        let rate_delta = percent_change(*before_rate, *after_rate);
        let rss_delta = percent_change(*before_rss, *after_rss);
        let mut reasons = Vec::new();
        if rate_delta < -max_throughput_regression {
            reasons.push("throughput");
        }
        if rss_delta > max_rss_regression {
            reasons.push("RSS");
        }
        let status = if reasons.is_empty() {
            "OK".to_owned()
        } else {
            format!("FAIL {}", reasons.join(","))
        };
        println!(
            "{}/{:<11} {rate_delta:>+14.2}% {rss_delta:>+15.2}%    {status}",
            key.0, key.1
        );
        if !reasons.is_empty() {
            failures.push(format!(
                "{}/{}: regressione {}",
                key.0,
                key.1,
                reasons.join(", ")
            ));
        }
    }
    if failures.is_empty() {
        println!("Confronto superato.");
        Ok(())
    } else {
        Err(format!("VETO PRESTAZIONALE:\n- {}", failures.join("\n- ")))
    }
}

// `main` è il driver dell'harness: parsing degli argomenti, orchestrazione dei
// sottoprocessi e scrittura della baseline condividono lo stesso stato e lo
// stesso ordine di esecuzione.
#[allow(clippy::too_many_lines)]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("compare") {
        let before = args.get(2).map(PathBuf::from);
        let after = args.get(3).map(PathBuf::from);
        let Some((before, after)) = before.zip(after) else {
            eprintln!("uso: plenora-bench compare <before.json> <after.json>");
            std::process::exit(2);
        };
        if let Err(error) = compare_baselines(&before, &after) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    let rows: usize = std::env::var("PLENORA_BENCH_ROWS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000_000);
    let deadline = Duration::from_secs(
        std::env::var("PLENORA_BENCH_DEADLINE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(900),
    );
    let repetitions: usize = std::env::var("PLENORA_BENCH_REPETITIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|count| *count > 0)
        .unwrap_or(1);

    if args.get(1).map(String::as_str) == Some("run") {
        let get = |k: &str| {
            args.iter()
                .position(|a| a == k)
                .and_then(|i| args.get(i + 1))
                .cloned()
                .unwrap_or_default()
        };
        println!("{}", run_one(&get("--driver"), &get("--op"), rows));
        return;
    }

    let exe = std::env::current_exe().unwrap();
    std::fs::create_dir_all(fixture_dir()).ok();
    let configured_drivers = std::env::var("PLENORA_BENCH_DRIVERS")
        .unwrap_or_else(|_| "geoparquet,geojson,csv,gpkg".to_owned());
    let drivers = configured_drivers
        .split(',')
        .map(str::trim)
        .filter(|driver| !driver.is_empty())
        .collect::<Vec<_>>();
    for driver in &drivers {
        match *driver {
            "geoparquet" | "geojson" | "csv" | "gpkg" | "kml" | "dxf" | "shp" | "xlsx" => {}
            other => panic!("PLENORA_BENCH_DRIVERS contiene un driver sconosciuto: {other}"),
        }
    }
    let mut results = Vec::new();
    for d in drivers {
        eprintln!("[{d}] prepare…");
        let prep = run_child(&exe, &["run", "--driver", d, "--op", "prepare"], deadline);
        if prep.get("prepared").is_none() {
            eprintln!("[{d}] prepare FALLITO: {prep}");
            let mut r = prep.clone();
            r["driver"] = d.into();
            r["op"] = "read".into();
            results.push(r);
        } else {
            for sample in 1..=repetitions {
                eprintln!("[{d}] read {sample}/{repetitions}");
                let mut r = run_child(&exe, &["run", "--driver", d, "--op", "read"], deadline);
                r["driver"] = d.into();
                r["op"] = "read".into();
                r["sample"] = sample.into();
                eprintln!("  read: {}", short(&r));
                results.push(r);
            }
        }
        for sample in 1..=repetitions {
            eprintln!("[{d}] write {sample}/{repetitions}");
            let mut wj = run_child(&exe, &["run", "--driver", d, "--op", "write"], deadline);
            wj["driver"] = d.into();
            wj["op"] = "write".into();
            wj["sample"] = sample.into();
            eprintln!("  write: {}", short(&wj));
            results.push(wj);
        }
        remove_dataset(d, &fixture_path(d));
    }

    let baseline = serde_json::json!({
        "contract": "plenora-io-baseline-v1",
        "source_revision": std::env::var("PLENORA_BENCH_SOURCE_REVISION")
            .unwrap_or_else(|_| "unknown".to_owned()),
        "geometry": if use_polygon() { "polygon" } else { "point" },
        "rows_per_benchmark": rows,
        "repetitions": repetitions,
        "drivers": configured_drivers,
        "note": "Driver eager/materializzanti possono fallire per OOM: e' un finding, non un bug dell'harness. Confrontare mediane su build e host identici. bytes_copied e metriche di coda: n/a in v1.",
        "benchmarks": results,
    });
    let output = PathBuf::from(
        std::env::var("PLENORA_BENCH_OUTPUT")
            .unwrap_or_else(|_| "baseline/baseline.json".to_owned()),
    );
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&output, serde_json::to_string_pretty(&baseline).unwrap()).unwrap();
    eprintln!("--- baseline scritta in {} ---", output.display());
}

// `rows_per_s` è un tasso non negativo già arrotondato: il cast a u64 serve
// solo alla riga di log, non alimenta la baseline.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn short(v: &serde_json::Value) -> String {
    if v.get("status").and_then(|s| s.as_str()) == Some("ok") {
        format!(
            "wall {:.0}ms  {} r/s  RSS {}MB  alloc {}  dec/enc {}/{}",
            v["wall_ms"].as_f64().unwrap_or(0.0),
            v["rows_per_s"].as_f64().unwrap_or(0.0) as u64,
            v["peak_rss_bytes"].as_u64().unwrap_or(0) / 1_048_576,
            v["allocation_count"].as_u64().unwrap_or(0),
            v["wkb_decode_count"].as_u64().unwrap_or(0),
            v["wkb_encode_count"].as_u64().unwrap_or(0),
        )
    } else {
        format!(
            "FAILED: {}",
            v.get("reason").and_then(|r| r.as_str()).unwrap_or("?")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_wgs84_drivers_use_crs84_in_schema_and_contract() {
        for driver in ["geojson", "kml"] {
            assert_eq!(bench_crs(driver), driver_common::OGC_CRS84);
            assert_eq!(
                bench_contract(driver).geometry.unwrap().crs.id(),
                Some(driver_common::OGC_CRS84)
            );
        }

        assert_eq!(bench_crs("csv"), "EPSG:4326");
        assert_eq!(
            bench_contract("csv").geometry.unwrap().crs.id(),
            Some("EPSG:4326")
        );
    }
}
