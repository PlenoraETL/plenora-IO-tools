//! plenora-io — CLI (Fase 2A). Comandi: `catalog` (registro driver), `inspect`
//! (formato + layer + schema + CRS), `layers` (elenco layer), `read` (scan +
//! conteggio righe), `convert` (pipeline read→write in **streaming**: apre il
//! driver sorgente, crea il driver destinazione, trasferisce i RecordBatch a
//! memoria O(batch), publish atomico). Nessuna riproiezione: il CRS è
//! letto/scritto, mai trasformato (ADR-IO 4).
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use plenora_core::contract::{DataContract, LayerContract};
use plenora_core::geometry::is_geometry_field;
use plenora_core::limits::Limits;
use plenora_io_core::driver::{FormatDriver, ReadOptions, Sink, Source, WriteOptions};
use plenora_io_core::publish::PublishOutcome;
use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadRequest};
use plenora_io_core::{DriverRegistry, WriteLayer, WritePlan};

/// Errore CLI: (exit code, documento JSON d'errore).
type CliResult = Result<Value, (i32, Value)>;

fn err_doc(code: &str, message: impl Into<String>) -> Value {
    json!({
        "status": "error",
        "protocol_version": 1,
        "contract": "plenora-io-error-v1",
        "error": {"code": code, "message": message.into()},
    })
}

fn usage_err(message: impl Into<String>) -> (i32, Value) {
    (2, err_doc("CLI_USAGE", message))
}

/// Mappa un `PlenoraError` a (exit, doc) con codici stabili.
fn map_err(e: plenora_core::PlenoraError) -> (i32, Value) {
    use plenora_core::PlenoraError as E;
    let (exit, code) = match &e {
        E::OutputExists(_) => (3, "OUTPUT_EXISTS"),
        E::Unsupported(_) => (4, "UNSUPPORTED"),
        E::Crs(_) => (5, "CRS_REQUIRED"),
        E::Contract(_) | E::Schema(_) => (6, "CONTRACT"),
        E::LimitExceeded(_) => (7, "LIMIT_EXCEEDED"),
        _ => (1, "FORMAT_ERROR"),
    };
    (exit, err_doc(code, e.to_string()))
}

// --- selezione driver per estensione --------------------------------------

fn driver_for_path(path: &Path) -> Result<Box<dyn FormatDriver>, (i32, Value)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let d: Box<dyn FormatDriver> = match ext.as_str() {
        "parquet" => Box::new(driver_geoparquet::GeoParquetDriver),
        "geojson" | "json" => Box::new(driver_geojson::GeoJsonDriver),
        "csv" => Box::new(driver_csv::CsvDriver),
        "gpkg" => Box::new(driver_gpkg::GpkgDriver),
        "shp" => Box::new(driver_shp::ShpDriver),
        "kml" => Box::new(driver_kml::KmlDriver),
        "xlsx" | "xls" => Box::new(driver_xls::XlsDriver),
        "dxf" => Box::new(driver_dxf::DxfDriver),
        "gdb" => Box::new(driver_filegdb::FileGdbDriver),
        "arrow" => Box::new(driver_ipc::IpcDriver),
        other => {
            return Err((
                4,
                err_doc(
                    "UNSUPPORTED",
                    format!("estensione non riconosciuta: '.{other}'"),
                ),
            ))
        }
    };
    Ok(d)
}

// --- parsing argomenti ------------------------------------------------------

#[derive(Default)]
struct Cli {
    positionals: Vec<String>,
    assume_crs: Option<String>,
    layer: Option<u32>,
    limit: Option<usize>,
    durable: bool,
    opts: BTreeMap<String, String>,
    in_opts: BTreeMap<String, String>,
    out_opts: BTreeMap<String, String>,
    limits: Limits,
}

fn kv(s: &str) -> Result<(String, String), (i32, Value)> {
    s.split_once('=')
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .ok_or_else(|| usage_err(format!("opzione '{s}' non è nel formato chiave=valore")))
}

fn parse_usize(value: Option<&String>, flag: &str) -> Result<usize, (i32, Value)> {
    value
        .ok_or_else(|| usage_err(format!("{flag} richiede un valore")))?
        .parse()
        .map_err(|_| usage_err(format!("{flag} richiede un intero non negativo")))
}

fn parse_u64(value: Option<&String>, flag: &str) -> Result<u64, (i32, Value)> {
    value
        .ok_or_else(|| usage_err(format!("{flag} richiede un valore")))?
        .parse()
        .map_err(|_| usage_err(format!("{flag} richiede un intero non negativo")))
}

fn parse(args: &[String]) -> Result<Cli, (i32, Value)> {
    let mut cli = Cli::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--assume-crs" => {
                cli.assume_crs = Some(
                    it.next()
                        .ok_or_else(|| usage_err("--assume-crs richiede un valore"))?
                        .clone(),
                )
            }
            "--layer" => {
                let v = it
                    .next()
                    .ok_or_else(|| usage_err("--layer richiede un valore"))?;
                cli.layer = Some(
                    v.parse()
                        .map_err(|_| usage_err("--layer richiede un intero"))?,
                );
            }
            "--limit" => {
                let v = it
                    .next()
                    .ok_or_else(|| usage_err("--limit richiede un valore"))?;
                cli.limit = Some(
                    v.parse()
                        .map_err(|_| usage_err("--limit richiede un intero"))?,
                );
            }
            "--max-input-bytes" => {
                cli.limits.max_input_bytes = parse_u64(it.next(), "--max-input-bytes")?;
            }
            "--max-output-bytes" => {
                cli.limits.max_output_bytes = parse_u64(it.next(), "--max-output-bytes")?;
            }
            "--max-rows" => {
                cli.limits.max_rows = parse_usize(it.next(), "--max-rows")?;
            }
            "--max-columns" => {
                cli.limits.max_columns = parse_usize(it.next(), "--max-columns")?;
            }
            "--max-vertices" => {
                cli.limits.max_vertices = parse_usize(it.next(), "--max-vertices")?;
            }
            "--max-wkb-cell-bytes" => {
                cli.limits.wkb.max_cell_bytes = parse_usize(it.next(), "--max-wkb-cell-bytes")?;
            }
            "--max-wkb-components" => {
                cli.limits.wkb.max_components = parse_usize(it.next(), "--max-wkb-components")?;
            }
            "--max-wkb-depth" => {
                cli.limits.wkb.max_depth = parse_usize(it.next(), "--max-wkb-depth")?;
            }
            "--durable" => cli.durable = true,
            "--opt" => {
                let (k, v) = kv(it
                    .next()
                    .ok_or_else(|| usage_err("--opt richiede chiave=valore"))?)?;
                cli.opts.insert(k, v);
            }
            "--in-opt" => {
                let (k, v) = kv(it
                    .next()
                    .ok_or_else(|| usage_err("--in-opt richiede chiave=valore"))?)?;
                cli.in_opts.insert(k, v);
            }
            "--out-opt" => {
                let (k, v) = kv(it
                    .next()
                    .ok_or_else(|| usage_err("--out-opt richiede chiave=valore"))?)?;
                cli.out_opts.insert(k, v);
            }
            other if other.starts_with("--") => {
                return Err(usage_err(format!("opzione sconosciuta: {other}")))
            }
            _ => cli.positionals.push(a.clone()),
        }
    }
    Ok(cli)
}

// --- rappresentazione JSON --------------------------------------------------

fn layer_json(l: &LayerContract) -> Value {
    let fields: Vec<Value> = l
        .contract
        .schema
        .fields()
        .iter()
        .map(|f| {
            json!({
                "name": f.name(),
                "type": format!("{:?}", f.data_type()),
                "nullable": f.is_nullable(),
                "geometry": is_geometry_field(f),
            })
        })
        .collect();
    let geom = l.contract.geometry.as_ref().map(|g| {
        json!({
            "name": g.name,
            "crs": g.crs.id(),
            "crs_resolution": &g.crs,
            "kind": g
                .resolved_crs()
                .map(|crs| format!("{:?}", crs.kind))
                .unwrap_or_else(|| "Unresolved".to_owned()),
        })
    });
    json!({
        "id": l.id.0,
        "name": l.name,
        "geometry": geom,
        "fields": fields,
    })
}

fn read_options(cli: &Cli) -> ReadOptions {
    ReadOptions {
        assume_crs: cli.assume_crs.clone(),
        format_options: cli.opts.clone(),
        limits: cli.limits,
    }
}

// --- comandi ----------------------------------------------------------------

fn cmd_catalog() -> CliResult {
    let mut registry = DriverRegistry::new();
    registry.register(Box::new(driver_geoparquet::GeoParquetDriver));
    registry.register(Box::new(driver_geojson::GeoJsonDriver));
    registry.register(Box::new(driver_csv::CsvDriver));
    registry.register(Box::new(driver_gpkg::GpkgDriver));
    registry.register(Box::new(driver_shp::ShpDriver));
    registry.register(Box::new(driver_kml::KmlDriver));
    registry.register(Box::new(driver_xls::XlsDriver));
    registry.register(Box::new(driver_dxf::DxfDriver));
    registry.register(Box::new(driver_filegdb::FileGdbDriver));
    registry.register(Box::new(driver_ipc::IpcDriver));
    let drivers = serde_json::to_value(registry.descriptors()).unwrap_or(Value::Null);
    Ok(json!({
        "status": "ok",
        "protocol_version": 1,
        "contract": "plenora-io-catalog-v1",
        "drivers": drivers,
    }))
}

fn open_source(cli: &Cli) -> Result<(Box<dyn FormatDriver>, PathBuf), (i32, Value)> {
    let path = PathBuf::from(
        cli.positionals
            .first()
            .ok_or_else(|| usage_err("manca il percorso del file"))?,
    );
    let driver = driver_for_path(&path)?;
    Ok((driver, path))
}

fn cmd_inspect(cli: &Cli) -> CliResult {
    let (driver, path) = open_source(cli)?;
    let ds = driver
        .open(Source::Path(path), &read_options(cli))
        .map_err(map_err)?;
    let fidelity = ds.fidelity_assessment();
    let layers: Vec<Value> = ds.layers().iter().map(layer_json).collect();
    Ok(json!({
        "status": "ok",
        "protocol_version": 1,
        "contract": "plenora-io-inspect-v1",
        "format": serde_json::to_value(driver.descriptor()).unwrap_or(Value::Null),
        "fidelity": fidelity,
        "layers": layers,
    }))
}

fn cmd_layers(cli: &Cli) -> CliResult {
    let (driver, path) = open_source(cli)?;
    let ds = driver
        .open(Source::Path(path), &read_options(cli))
        .map_err(map_err)?;
    let fidelity = ds.fidelity_assessment();
    let layers: Vec<Value> = ds
        .layers()
        .iter()
        .map(|l| {
            json!({
                "id": l.id.0,
                "name": l.name,
                "geometry_crs": l
                    .contract
                    .geometry
                    .as_ref()
                    .and_then(|g| g.crs.id().map(str::to_owned)),
                "field_count": l.contract.schema.fields().len(),
            })
        })
        .collect();
    Ok(json!({
        "status": "ok",
        "protocol_version": 1,
        "contract": "plenora-io-layers-v1",
        "format": driver.descriptor().id,
        "fidelity": fidelity,
        "layers": layers,
    }))
}

fn read_request(layer_id: u32) -> ReadRequest {
    ReadRequest {
        layer: plenora_core::contract::LayerId(layer_id),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        batch_target: BatchTarget::default(),
    }
}

fn cmd_read(cli: &Cli) -> CliResult {
    let (driver, path) = open_source(cli)?;
    let ds = driver
        .open(Source::Path(path), &read_options(cli))
        .map_err(map_err)?;
    let fidelity = ds.fidelity_assessment();
    let layer_id = cli.layer.unwrap_or(0);
    let contract = ds
        .layers()
        .iter()
        .find(|l| l.id.0 == layer_id)
        .ok_or_else(|| {
            (
                1,
                err_doc("NO_LAYER", format!("layer {layer_id} inesistente")),
            )
        })?
        .clone();
    let mut reader = ds
        .open_layer_reader(&read_request(layer_id))
        .map_err(map_err)?;
    let (mut rows, mut batches) = (0usize, 0usize);
    while let Some(batch) = reader.next_batch().map_err(map_err)? {
        rows += batch.num_rows();
        batches += 1;
        if cli.limit.is_some_and(|l| rows >= l) {
            break;
        }
    }
    Ok(json!({
        "status": "ok",
        "protocol_version": 1,
        "contract": "plenora-io-read-v1",
        "format": driver.descriptor().id,
        "fidelity": fidelity,
        "layer": layer_json(&contract),
        "rows_read": rows,
        "batches": batches,
        "truncated": cli.limit.is_some_and(|l| rows >= l),
    }))
}

fn cmd_convert(cli: &Cli) -> CliResult {
    if cli.positionals.len() < 2 {
        return Err(usage_err("convert richiede <ingresso> <uscita>"));
    }
    let in_path = PathBuf::from(&cli.positionals[0]);
    let out_path = PathBuf::from(&cli.positionals[1]);
    let src = driver_for_path(&in_path)?;
    let dst = driver_for_path(&out_path)?;

    let ropts = ReadOptions {
        assume_crs: cli.assume_crs.clone(),
        format_options: cli.in_opts.clone(),
        limits: cli.limits,
    };
    let ds = src.open(Source::Path(in_path), &ropts).map_err(map_err)?;
    let read_fidelity = ds.fidelity_assessment();

    // Layer da convertire: `--layer` ne sceglie uno, altrimenti tutti.
    let all: Vec<LayerContract> = ds.layers().to_vec();
    let selected: Vec<LayerContract> = match cli.layer {
        Some(id) => vec![all
            .iter()
            .find(|l| l.id.0 == id)
            .cloned()
            .ok_or_else(|| (1, err_doc("NO_LAYER", format!("layer {id} inesistente"))))?],
        None => all,
    };
    // Multi-layer verso destinazione single-layer: vietato (fail-closed).
    if selected.len() > 1 && !dst.descriptor().multi_layer {
        return Err((
            4,
            err_doc(
                "SINGLE_LAYER_SINK",
                format!(
                    "sorgente con {} layer ma '{}' è single-layer: usa --layer N per sceglierne uno",
                    selected.len(),
                    dst.descriptor().id
                ),
            ),
        ));
    }

    let plan = WritePlan {
        layers: selected
            .iter()
            .map(|l| WriteLayer {
                name: l.name.clone(),
                contract: DataContract {
                    schema: l.contract.schema.clone(),
                    // Propaga il contratto geometria (id CRS + WKT) ai writer che
                    // ne hanno bisogno (gpkg srs, shp .prj); i writer che rilevano
                    // la geometria dallo schema lo ignorano.
                    geometry: l.contract.geometry.clone(),
                },
            })
            .collect(),
    };
    let wopts = WriteOptions {
        durable: cli.durable,
        format_options: cli.out_opts.clone(),
        limits: cli.limits,
    };
    let mut writer = dst
        .create(Sink::Path(out_path), &plan, &wopts)
        .map_err(map_err)?;

    // L'i-esimo layer sorgente scrive nel LayerId(i) del piano di destinazione.
    let mut layer_reports = Vec::new();
    let mut total_rows = 0usize;
    for (sink_idx, l) in selected.iter().enumerate() {
        let mut reader = ds
            .open_layer_reader(&read_request(l.id.0))
            .map_err(map_err)?;
        let (mut rows, mut batches) = (0usize, 0usize);
        while let Some(batch) = reader.next_batch().map_err(map_err)? {
            rows += batch.num_rows();
            batches += 1;
            writer
                .write_to_layer(plenora_core::contract::LayerId(sink_idx as u32), &batch)
                .map_err(map_err)?;
        }
        total_rows += rows;
        layer_reports.push(json!({"name": l.name, "rows": rows, "batches": batches}));
    }
    let published = writer.finish().map_err(map_err)?;

    let outcome = match published.outcome {
        PublishOutcome::Published => "published",
        PublishOutcome::PublishedButDurabilityUnconfirmed => "published_durability_unconfirmed",
    };
    Ok(json!({
        "status": "ok",
        "protocol_version": 1,
        "contract": "plenora-io-convert-v1",
        "from": src.descriptor().id,
        "to": dst.descriptor().id,
        "layers": layer_reports,
        "total_rows": total_rows,
        "bytes_written": published.bytes,
        "publish_outcome": outcome,
        "read_fidelity": read_fidelity,
        "write_fidelity": &published.fidelity,
        "loss": {
            "lossless": published.fidelity.level == plenora_io_core::Fidelity::Lossless
                && published.loss.is_empty(),
            "counts": serde_json::to_value(&published.loss.counts).unwrap_or(Value::Null),
        },
    }))
}

fn run() -> CliResult {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => Ok(json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
        })),
        Some("catalog") => cmd_catalog(),
        Some("inspect") => cmd_inspect(&parse(&args[1..])?),
        Some("layers") => cmd_layers(&parse(&args[1..])?),
        Some("read") => cmd_read(&parse(&args[1..])?),
        Some("convert") => cmd_convert(&parse(&args[1..])?),
        _ => Err(usage_err(
            "uso: plenora-io <catalog|inspect|layers|read|convert> [args] | --version",
        )),
    }
}

fn main() {
    match run() {
        Ok(doc) => println!("{doc}"),
        Err((exit, doc)) => {
            eprintln!("{doc}");
            std::process::exit(exit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_flags_and_opts() {
        let args = [
            "--assume-crs",
            "EPSG:4326",
            "in.csv",
            "--opt",
            "wkt_column=g",
            "--durable",
            "--layer",
            "2",
            "--max-rows",
            "123",
            "--max-wkb-depth",
            "9",
        ]
        .map(String::from)
        .to_vec();
        let cli = parse(&args).unwrap();
        assert_eq!(cli.assume_crs.as_deref(), Some("EPSG:4326"));
        assert_eq!(cli.positionals, vec!["in.csv".to_owned()]);
        assert_eq!(cli.opts.get("wkt_column").map(String::as_str), Some("g"));
        assert!(cli.durable);
        assert_eq!(cli.layer, Some(2));
        assert_eq!(cli.limits.max_rows, 123);
        assert_eq!(cli.limits.wkb.max_depth, 9);
    }

    #[test]
    fn kv_split() {
        assert_eq!(kv("a=b").unwrap(), ("a".to_owned(), "b".to_owned()));
        assert!(kv("nope").is_err());
    }

    #[test]
    fn ext_to_driver() {
        assert_eq!(
            driver_for_path(Path::new("x.geojson"))
                .unwrap()
                .descriptor()
                .id,
            "geojson"
        );
        assert_eq!(
            driver_for_path(Path::new("x.gpkg"))
                .unwrap()
                .descriptor()
                .id,
            "gpkg"
        );
        assert_eq!(
            driver_for_path(Path::new("x.parquet"))
                .unwrap()
                .descriptor()
                .id,
            "geoparquet"
        );
        assert!(driver_for_path(Path::new("x.zzz")).is_err());
    }
}

#[cfg(test)]
mod conformance_tests;
