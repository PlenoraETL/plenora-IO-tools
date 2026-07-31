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

use plenora_io_core::driver::{FormatDriver, ReadOptions, Sink, Source, WriteOptions};
use plenora_io_core::publish::PublishOutcome;
use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadRequest};
use plenora_io_core::{
    DriverRegistry, Fidelity, FidelityAssessment, LossReport, WriteLayer, WritePlan,
};
use plenora_io_model::contract::{DataContract, LayerContract};
use plenora_io_model::geometry::is_geometry_field;
use plenora_io_model::limits::Limits;
use plenora_io_model::{ErrorCategory, ErrorPhase, PlenoraIoError, RemoteEffect, RetryDisposition};

/// Errore CLI: (exit code, documento JSON d'errore).
type CliResult = Result<Value, (i32, Value)>;

fn err_doc(code: &str, error: &PlenoraIoError) -> Value {
    json!({
        "status": "error",
        "protocol_version": 1,
        "contract": "plenora-io-error-v1",
        "error": {
            "category": error.category,
            "phase": error.phase,
            "remote_effect": error.remote_effect,
            "retry": error.retry,
            "code": code,
            "message": error.message,
        },
    })
}

fn local_err_doc(
    code: &str,
    category: ErrorCategory,
    phase: ErrorPhase,
    message: impl Into<String>,
) -> Value {
    err_doc(
        code,
        &PlenoraIoError::new(
            category,
            phase,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        ),
    )
}

fn usage_err(message: impl Into<String>) -> (i32, Value) {
    (
        2,
        local_err_doc(
            "CLI_USAGE",
            ErrorCategory::InvalidConfiguration,
            ErrorPhase::Validate,
            message,
        ),
    )
}

/// Mappa un `PlenoraIoError` a (exit, doc) con codici stabili.
fn map_err(e: plenora_io_model::PlenoraIoError) -> (i32, Value) {
    use plenora_io_model::IoErrorCode;
    let (exit, code) = match e.code {
        IoErrorCode::OutputExists => (3, "OUTPUT_EXISTS"),
        IoErrorCode::Unsupported | IoErrorCode::Capability => (4, "UNSUPPORTED"),
        IoErrorCode::Crs => (5, "CRS_REQUIRED"),
        IoErrorCode::Contract | IoErrorCode::Schema => (6, "CONTRACT"),
        IoErrorCode::LimitExceeded => (7, "LIMIT_EXCEEDED"),
        IoErrorCode::ReaderBusy => (8, "READER_BUSY"),
        IoErrorCode::ProjectionUnsupported => (8, "PROJECTION_UNSUPPORTED"),
        IoErrorCode::CrsUnresolved => (8, "CRS_UNRESOLVED"),
        _ => (1, "FORMAT_ERROR"),
    };
    (exit, err_doc(code, &e))
}

fn combined_fidelity(read: &FidelityAssessment, write: &FidelityAssessment) -> FidelityAssessment {
    let level = match (read.level, write.level) {
        (Fidelity::Approximating, _) | (_, Fidelity::Approximating) => Fidelity::Approximating,
        (Fidelity::Conditional, _) | (_, Fidelity::Conditional) => Fidelity::Conditional,
        (Fidelity::Lossless, Fidelity::Lossless) => Fidelity::Lossless,
    };
    let mut combined = FidelityAssessment {
        level,
        reasons: Vec::new(),
    };
    for reason in read.reasons.iter().chain(&write.reasons) {
        combined.add_reason(reason.code, reason.detail.clone());
    }
    combined
}

fn loss_doc(fidelity: &FidelityAssessment, loss: &LossReport) -> Value {
    json!({
        "lossless": fidelity.level == Fidelity::Lossless && loss.is_empty(),
        "counts": serde_json::to_value(&loss.counts).unwrap_or(Value::Null),
    })
}

// --- selezione driver per estensione --------------------------------------

fn driver_for_path(path: &Path) -> Result<Box<dyn FormatDriver>, (i32, Value)> {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".shp.d"))
    {
        return Ok(Box::new(driver_shp::ShpDriver));
    }
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
                local_err_doc(
                    "UNSUPPORTED",
                    ErrorCategory::Unsupported,
                    ErrorPhase::Validate,
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
        resource_budget: Default::default(),
        cancellation: Default::default(),
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
        "determinism": "byte_for_byte",
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
        layer: plenora_io_model::contract::LayerId(layer_id),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        batch_target: BatchTarget::default(),
        cancellation: Default::default(),
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
                local_err_doc(
                    "NO_LAYER",
                    ErrorCategory::NotFound,
                    ErrorPhase::Prepare,
                    format!("layer {layer_id} inesistente"),
                ),
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
    let resource_budget = plenora_io_model::ResourceBudget::default();

    let ropts = ReadOptions {
        assume_crs: cli.assume_crs.clone(),
        format_options: cli.in_opts.clone(),
        limits: cli.limits,
        resource_budget: resource_budget.clone(),
        cancellation: Default::default(),
    };
    let ds = src.open(Source::Path(in_path), &ropts).map_err(map_err)?;
    let initial_read_fidelity = ds.fidelity_assessment();

    // Layer da convertire: `--layer` ne sceglie uno, altrimenti tutti.
    let all: Vec<LayerContract> = ds.layers().to_vec();
    let selected: Vec<LayerContract> = match cli.layer {
        Some(id) => vec![all.iter().find(|l| l.id.0 == id).cloned().ok_or_else(|| {
            (
                1,
                local_err_doc(
                    "NO_LAYER",
                    ErrorCategory::NotFound,
                    ErrorPhase::Prepare,
                    format!("layer {id} inesistente"),
                ),
            )
        })?],
        None => all,
    };
    // Multi-layer verso destinazione single-layer: vietato (fail-closed).
    if selected.len() > 1 && !dst.descriptor().multi_layer {
        return Err((
            4,
            local_err_doc(
                "SINGLE_LAYER_SINK",
                ErrorCategory::InvalidPlan,
                ErrorPhase::Validate,
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
        resource_budget,
        cancellation: Default::default(),
    };
    let mut writer = dst
        .create(Sink::Path(out_path), &plan, &wopts)
        .map_err(map_err)?;

    // L'i-esimo layer sorgente scrive nel LayerId(i) del piano di destinazione.
    let mut layer_reports = Vec::new();
    let mut total_rows = 0usize;
    let mut read_loss = LossReport::default();
    for (sink_idx, l) in selected.iter().enumerate() {
        let mut reader = ds
            .open_layer_reader(&read_request(l.id.0))
            .map_err(map_err)?;
        let (mut rows, mut batches) = (0usize, 0usize);
        while let Some(batch) = reader.next_batch().map_err(map_err)? {
            rows += batch.num_rows();
            batches += 1;
            writer
                .write_to_layer(plenora_io_model::contract::LayerId(sink_idx as u32), &batch)
                .map_err(map_err)?;
        }
        read_loss.merge(&reader.loss_report());
        total_rows += rows;
        layer_reports.push(json!({"name": l.name, "rows": rows, "batches": batches}));
    }
    let published = writer.finish().map_err(map_err)?;
    let read_fidelity = initial_read_fidelity.with_loss_report(&read_loss);
    let conversion_fidelity = combined_fidelity(&read_fidelity, &published.fidelity);

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
        "read_fidelity": &read_fidelity,
        "write_fidelity": &published.fidelity,
        "conversion_fidelity": conversion_fidelity,
        "read_loss": loss_doc(&read_fidelity, &read_loss),
        "write_loss": loss_doc(&published.fidelity, &published.loss),
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

    fn assert_candidate_envelope(name: &str, document: &Value) {
        let manifest: Value =
            serde_json::from_str(include_str!("../../../release/cli-protocol-v1.json")).unwrap();
        let envelope = &manifest["envelopes"][name];
        assert_eq!(document["contract"], envelope["contract"]);
        for field in envelope["required_top_level"].as_array().unwrap() {
            let field = field.as_str().unwrap();
            assert!(
                document.get(field).is_some(),
                "{name}: campo {field} assente"
            );
        }
        if let Some(forbidden) = envelope["forbidden_legacy_fields"].as_array() {
            for field in forbidden {
                let field = field.as_str().unwrap();
                assert!(
                    document.get(field).is_none(),
                    "{name}: campo legacy {field} presente"
                );
            }
        }
    }

    fn materialize_empty_ipc(directory: &tempfile::TempDir) -> PathBuf {
        use std::sync::Arc;

        use arrow_schema::{DataType, Field, Schema};

        let path = directory.path().join("input.arrow");
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "layer".to_owned(),
                contract: DataContract {
                    schema: Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)])),
                    geometry: None,
                },
            }],
        };
        let writer = driver_ipc::IpcDriver
            .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.finish().unwrap();
        path
    }

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
    fn reader_busy_has_stable_cli_error() {
        let (exit, document) = map_err(plenora_io_model::PlenoraIoError::reader_busy("kml", 0));
        assert_eq!(exit, 8);
        assert_eq!(document["error"]["code"], "READER_BUSY");
        assert_eq!(document["error"]["category"], "conflict");
        assert_eq!(document["error"]["phase"], "prepare");
        assert_eq!(document["error"]["remote_effect"], "none");
        assert_eq!(
            document["error"]["retry"],
            serde_json::json!({"kind": "never"})
        );
    }

    #[test]
    fn retry_after_keeps_delay_in_the_cli_envelope() {
        let error = PlenoraIoError::new(
            ErrorCategory::Transient,
            ErrorPhase::Connect,
            RemoteEffect::None,
            RetryDisposition::After(2_750),
            "servizio temporaneamente non disponibile",
        );
        let (exit, document) = map_err(error);

        assert_eq!(exit, 1);
        assert_candidate_envelope("error", &document);
        assert_eq!(
            document,
            serde_json::json!({
                "status": "error",
                "protocol_version": 1,
                "contract": "plenora-io-error-v1",
                "error": {
                    "category": "transient",
                    "phase": "connect",
                    "remote_effect": "none",
                    "retry": {"kind": "after", "delay_ms": 2_750},
                    "code": "FORMAT_ERROR",
                    "message": "servizio temporaneamente non disponibile",
                },
            })
        );
    }

    #[test]
    fn usage_errors_also_expose_machine_readable_axes() {
        let (exit, document) = usage_err("argomento mancante");
        assert_eq!(exit, 2);
        assert_eq!(document["error"]["category"], "invalid_configuration");
        assert_eq!(document["error"]["phase"], "validate");
        assert_eq!(
            document["error"]["retry"],
            serde_json::json!({"kind": "never"})
        );
        assert_eq!(document["error"]["message"], "argomento mancante");
    }

    #[test]
    fn convert_observability_separates_read_write_and_end_to_end_fidelity() {
        let mut read_loss = LossReport::default();
        read_loss.record("inconsistent_crs_representations", 1);
        let read = FidelityAssessment::lossless().with_loss_report(&read_loss);
        let write = FidelityAssessment::lossless();
        let conversion = combined_fidelity(&read, &write);

        assert_eq!(conversion.level, Fidelity::Approximating);
        assert_eq!(
            loss_doc(&read, &read_loss),
            serde_json::json!({
                "lossless": false,
                "counts": {"inconsistent_crs_representations": 1},
            })
        );
        assert_eq!(
            loss_doc(&write, &LossReport::default()),
            serde_json::json!({"lossless": true, "counts": {}})
        );
    }

    #[test]
    fn combined_fidelity_uses_the_worst_level_and_bounds_reasons() {
        let mut read = FidelityAssessment::for_format("shp", Fidelity::Conditional);
        for index in 0..plenora_io_core::MAX_FIDELITY_REASONS {
            read.add_reason(
                plenora_io_core::FidelityReasonCode::FormatConstraint,
                format!("read-{index}"),
            );
        }
        let write = FidelityAssessment::for_format("dxf", Fidelity::Approximating);
        let combined = combined_fidelity(&read, &write);

        assert_eq!(combined.level, Fidelity::Approximating);
        assert_eq!(
            combined.reasons.len(),
            plenora_io_core::MAX_FIDELITY_REASONS
        );
    }

    #[test]
    fn convert_exposes_reader_crs_inconsistency_without_writer_ambiguity() {
        use std::collections::HashMap;
        use std::sync::Arc;

        use arrow_schema::{DataType, Field, Schema};
        use plenora_io_model::contract::{FieldId, GeometryColumnContract, GeometryType};
        use plenora_io_model::crs::{CrsKind, CrsResolution, ResolvedCrs};
        use plenora_io_model::geometry::{
            with_geometry_contract_metadata, ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION,
        };

        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input.arrow");
        let output = directory.path().join("output.arrow");
        let mut geometry = GeometryColumnContract::wkb_xy(
            FieldId(0),
            "geometry",
            CrsResolution::resolved(ResolvedCrs::new(
                Some("EPSG:4326".to_owned()),
                CrsKind::Geographic,
                None,
            )),
            true,
        );
        geometry.srid = Some(3003);
        geometry.set_exact_geometry_types(vec![GeometryType::Point]);
        let base = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([(
            ARROW_EXTENSION_NAME_KEY.to_owned(),
            GEOARROW_WKB_EXTENSION.to_owned(),
        )]));
        let schema = Arc::new(Schema::new(vec![with_geometry_contract_metadata(
            &base, &geometry,
        )]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "conflicting_crs".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry),
                },
            }],
        };
        let driver = driver_ipc::IpcDriver;
        let writer = driver
            .create(Sink::Path(input.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.finish().unwrap();

        let cli = parse(&[
            input.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
        ])
        .unwrap();
        let document = cmd_convert(&cli).unwrap();

        assert_candidate_envelope("convert", &document);
        assert_eq!(
            document["read_loss"],
            serde_json::json!({
                "lossless": false,
                "counts": {"inconsistent_crs_representations": 1},
            })
        );
        assert_eq!(
            document["write_loss"],
            serde_json::json!({"lossless": true, "counts": {}})
        );
        assert_eq!(document["conversion_fidelity"]["level"], "approximating");

        let reopened = driver
            .open(
                Source::Path(output),
                &plenora_io_core::ReadOptions::default(),
            )
            .unwrap();
        let reopened_geometry = reopened.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(reopened_geometry.crs.id(), Some("EPSG:4326"));
        assert_eq!(reopened_geometry.srid, Some(3003));

        let shapefile_output = directory.path().join("must_not_exist.shp");
        let cli = parse(&[
            input.to_string_lossy().into_owned(),
            shapefile_output.to_string_lossy().into_owned(),
        ])
        .unwrap();
        let (exit, error) = cmd_convert(&cli).unwrap_err();
        assert_eq!(exit, 4);
        assert_eq!(error["error"]["category"], "unsupported");
        assert_eq!(error["error"]["phase"], "validate");
        assert_eq!(error["error"]["remote_effect"], "none");
        assert_eq!(error["error"]["retry"]["kind"], "never");
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("rappresentazioni CRS discordanti"));
        assert!(!shapefile_output.exists());
    }

    #[test]
    fn projection_unsupported_has_stable_cli_error() {
        let (exit, document) = map_err(plenora_io_model::PlenoraIoError::projection_unsupported(
            "csv",
        ));
        assert_eq!(exit, 8);
        assert_eq!(document["error"]["code"], "PROJECTION_UNSUPPORTED");
    }

    #[test]
    fn unresolved_crs_has_stable_redacted_cli_error() {
        let raw = plenora_io_model::crs::RawCrs::new(
            "LOCAL_CS[\"survey-grid-secret\"]".to_owned(),
            Some("authority-secret".to_owned()),
        );
        let (_, document) = map_err(plenora_io_model::PlenoraIoError::crs_unresolved(
            "shp", &raw,
        ));
        assert_eq!(document["error"]["code"], "CRS_UNRESOLVED");
        assert!(!document.to_string().contains("survey-grid-secret"));
        assert!(!document.to_string().contains("authority-secret"));
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
        assert_eq!(
            driver_for_path(Path::new("x.shp.d"))
                .unwrap()
                .descriptor()
                .id,
            "shp"
        );
        assert!(driver_for_path(Path::new("x.zzz")).is_err());
    }

    #[test]
    fn catalog_is_canonical_and_byte_for_byte_deterministic() {
        let first = serde_json::to_vec(&cmd_catalog().unwrap()).unwrap();
        let second = serde_json::to_vec(&cmd_catalog().unwrap()).unwrap();
        assert_eq!(first, second);

        let document: Value = serde_json::from_slice(&first).unwrap();
        assert_candidate_envelope("catalog", &document);
        assert_eq!(document["determinism"], "byte_for_byte");
        let ids: Vec<_> = document["drivers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|driver| driver["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            vec![
                "csv",
                "dxf",
                "filegdb",
                "geojson",
                "geoparquet",
                "gpkg",
                "ipc",
                "kml",
                "shp",
                "xls",
            ]
        );
    }

    #[test]
    fn inspect_layers_and_read_match_the_candidate_protocol_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let input = materialize_empty_ipc(&directory);
        let cli = parse(&[input.to_string_lossy().into_owned()]).unwrap();

        assert_candidate_envelope("inspect", &cmd_inspect(&cli).unwrap());
        assert_candidate_envelope("layers", &cmd_layers(&cli).unwrap());
        assert_candidate_envelope("read", &cmd_read(&cli).unwrap());
    }
}

#[cfg(test)]
mod conformance_tests;
