//! plenora-bench — harness di baseline prestazionale. NON ottimizza: misura
//! throughput, picco RSS, allocazioni e decode/encode WKB per driver, e archivia
//! una baseline. Metriche di `Prestazioni.md` §7 (bytes_copied e metriche di
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
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arrow_array::{Array, BinaryArray, Float64Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};

use driver_common::geometry_field;
use plenora_io_core::driver::{FormatDriver, ReadOptions, Sink, Source, WriteOptions};
use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadRequest};
use plenora_io_core::{WriteLayer, WritePlan};
use plenora_io_model::contract::{DataContract, LayerId};
use plenora_io_model::wkb::to_wkb;

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
        libc::getrusage(libc::RUSAGE_SELF, &mut u);
    }
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
        .map(|kb| kb * 1024)
        .unwrap_or(0)
}

// --- generazione a chunk (pool WKB, niente encode nella generazione) -------

fn bench_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        geometry_field("geometry", "EPSG:4326"),
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("val", DataType::Float64, false),
    ]))
}

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

fn gen_chunk(pool: &[Vec<u8>], names: &[String], start: usize, count: usize) -> RecordBatch {
    let geom = BinaryArray::from(
        (0..count)
            .map(|j| Some(pool[(start + j) % POOL].as_slice()))
            .collect::<Vec<_>>(),
    );
    let ids = Int64Array::from((0..count).map(|j| (start + j) as i64).collect::<Vec<_>>());
    let nm = StringArray::from(
        (0..count)
            .map(|j| names[(start + j) % POOL].as_str())
            .collect::<Vec<_>>(),
    );
    let vals = Float64Array::from(
        (0..count)
            .map(|j| (start + j) as f64 * 1.5)
            .collect::<Vec<_>>(),
    );
    RecordBatch::try_new(
        bench_schema(),
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
        other => panic!("driver sconosciuto: {other}"),
    }
}

fn ext(id: &str) -> &'static str {
    match id {
        "geoparquet" => "parquet",
        "geojson" => "geojson",
        "csv" => "csv",
        "gpkg" => "gpkg",
        _ => "bin",
    }
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(std::env::var("PLENORA_BENCH_FIXDIR").unwrap_or_else(|_| "bench-fix".to_owned()))
}

fn fixture_path(id: &str) -> PathBuf {
    fixture_dir().join(format!("{id}.{}", ext(id)))
}

fn read_opts(id: &str) -> ReadOptions {
    let mut o = ReadOptions::default();
    if id == "csv" {
        o.assume_crs = Some("EPSG:4326".to_owned());
        o.format_options
            .insert("wkt_column".to_owned(), "geometry".to_owned());
    }
    o
}

/// Scrittura a chunk via driver (streaming se il driver lo è; usato per i
/// fixture geoparquet/gpkg e per il benchmark di `write`). Ritorna
/// (n_batch, max_batch_bytes).
fn feed_write(
    id: &str,
    path: &Path,
    total: usize,
    pool: &[Vec<u8>],
    names: &[String],
) -> (usize, usize) {
    if path.exists() {
        std::fs::remove_file(path).ok();
    }
    let driver = driver_by_id(id);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "bench".to_owned(),
            contract: DataContract {
                schema: bench_schema(),
                geometry: None,
            },
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
        let batch = gen_chunk(pool, names, start, c);
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
        write!(
            w,
            "{{\"type\":\"Feature\",\"geometry\":{geom},\"properties\":{{\"id\":{i},\"name\":\"f{}\",\"val\":{}}}}}",
            i % POOL,
            i as f64 * 1.5
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
        writeln!(
            w,
            "{i},f{},{},\"POINT ({x} {y})\"",
            i % POOL,
            i as f64 * 1.5
        )
        .unwrap();
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
        .open(Source::Path(path.to_owned()), &read_opts(id))
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
            batch_target: BatchTarget::default(),
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
        return serde_json::json!({"prepared": id, "bytes": file_len(&path)});
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
    if op == "read" || op == "read_proj" || op == "read_pruned" {
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
        io_bytes = file_len(&path);
    } else {
        let out = fixture_dir().join(format!("{id}-out.{}", ext(id)));
        std::fs::create_dir_all(fixture_dir()).ok();
        let (nb, mbb) = feed_write(id, &out, rows, &pool, &names);
        io_bytes = file_len(&out);
        std::fs::remove_file(&out).ok();
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
    let mb = io_bytes as f64 / 1_048_576.0;
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
        "rows_per_s": if secs > 0.0 { (rows_done as f64 / secs).round() } else { 0.0 },
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
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > deadline {
                    child.kill().ok();
                    timed_out = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => break,
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
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
    let drivers = ["geoparquet", "geojson", "csv", "gpkg"];
    let mut results = Vec::new();
    for d in drivers {
        eprintln!("[{d}] prepare…");
        let prep = run_child(&exe, &["run", "--driver", d, "--op", "prepare"], deadline);
        let prepared = prep.get("prepared").is_some();
        if !prepared {
            eprintln!("[{d}] prepare FALLITO: {prep}");
            let mut r = prep.clone();
            r["driver"] = d.into();
            r["op"] = "read".into();
            results.push(r);
        } else {
            eprintln!("[{d}] read…");
            let mut r = run_child(&exe, &["run", "--driver", d, "--op", "read"], deadline);
            r["driver"] = d.into();
            r["op"] = "read".into();
            eprintln!("  read: {}", short(&r));
            results.push(r);
        }
        eprintln!("[{d}] write…");
        let mut wj = run_child(&exe, &["run", "--driver", d, "--op", "write"], deadline);
        wj["driver"] = d.into();
        wj["op"] = "write".into();
        eprintln!("  write: {}", short(&wj));
        results.push(wj);
        std::fs::remove_file(fixture_path(d)).ok();
    }

    let baseline = serde_json::json!({
        "contract": "plenora-io-baseline-v1",
        "rows_per_benchmark": rows,
        "note": "Baseline 10M righe. Driver eager/materializzanti falliscono per OOM (finding, non bug dell'harness). Riferimento per il budget di regressione. bytes_copied e metriche di coda: n/a in v1.",
        "benchmarks": results,
    });
    std::fs::create_dir_all("baseline").ok();
    std::fs::write(
        "baseline/baseline.json",
        serde_json::to_string_pretty(&baseline).unwrap(),
    )
    .unwrap();
    eprintln!("--- baseline scritta in baseline/baseline.json ---");
}

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
