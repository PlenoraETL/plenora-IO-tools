//! plenora-io — CLI (Fase 2A). Comandi: `catalog` (registro driver), `inspect`
//! (formato + layer + schema + CRS), `layers` (elenco layer), `read` (scan +
//! conteggio righe), `convert` (pipeline operation-atomic: valida la sorgente
//! fino a EOF prima di esporre batch al writer, poi trasferisce i `RecordBatch`
//! e pubblica atomicamente). Nessuna riproiezione: il CRS è
//! letto/scritto, mai trasformato (ADR-IO 4).
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use plenora_io_core::driver::{FormatDriver, ReadOptions, Sink, Source, WriteOptions};
use plenora_io_core::publish::PublishOutcome;
use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadRequest, ReadScope};
use plenora_io_core::{
    DriverRegistry, Fidelity, FidelityAssessment, LossReport, WriteLayer, WritePlan,
};
use plenora_io_model::contract::{DataContract, LayerContract};
use plenora_io_model::geometry::is_geometry_field;
use plenora_io_model::limits::Limits;
use plenora_io_model::{
    CancellationToken, ErrorCategory, ErrorPhase, PlenoraIoError, RemoteEffect, ResourceBudget,
    ResourceLimits, RetryDisposition,
};

/// Errore CLI: (exit code, documento JSON d'errore).
type CliResult = Result<Value, (i32, Value)>;

fn err_doc(code: &str, error: &PlenoraIoError) -> Value {
    let mut error_document = json!({
        "category": error.category,
        "phase": error.phase,
        "remote_effect": error.remote_effect,
        "retry": error.retry,
        "code": code,
        "message": error.message,
    });
    if let Some(diagnostics) = &error.row_diagnostics {
        match serde_json::to_value(diagnostics.as_ref()) {
            Ok(document) => error_document["row_diagnostics"] = document,
            Err(_) => {
                error_document = json!({
                    "category": ErrorCategory::Internal,
                    "phase": ErrorPhase::Validate,
                    "remote_effect": RemoteEffect::None,
                    "retry": RetryDisposition::Never,
                    "code": "INVALID_ROW_DIAGNOSTICS",
                    "message": "diagnostica row-scoped interna non conforme e non emessa",
                });
            }
        }
    }
    json!({
        "status": "error",
        "protocol_version": 1,
        "contract": "plenora-io-error-v1",
        "error": error_document,
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
// Usata come funzione in `map_err`/`unwrap_or_else`: la firma per valore è
// imposta dai punti di chiamata.
#[allow(clippy::needless_pass_by_value)]
fn map_err(e: plenora_io_model::PlenoraIoError) -> (i32, Value) {
    use plenora_io_model::IoErrorCode;
    let (historical_exit, code) = match e.code {
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
    let (exit, code) = if e.category == ErrorCategory::Cancelled {
        (130, "CANCELLED")
    } else if e.category == ErrorCategory::DataMapping {
        (2, code)
    } else {
        (historical_exit, code)
    };
    let document = err_doc(code, &e);
    let final_exit = if document["error"]["code"] == "INVALID_ROW_DIAGNOSTICS" {
        1
    } else {
        exit
    };
    (final_exit, document)
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
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let d: Box<dyn FormatDriver> = match ext.as_str() {
        "parquet" => Box::new(driver_geoparquet::GeoParquetDriver),
        "geojson" | "json" => Box::new(driver_geojson::GeoJsonDriver),
        "csv" => Box::new(driver_csv::CsvDriver),
        "gpkg" => Box::new(driver_gpkg::GpkgDriver),
        "shp" => Box::new(driver_shp::ShpDriver),
        "kml" => Box::new(driver_kml::KmlDriver),
        "xlsx" => Box::new(driver_xls::XlsDriver),
        "xls" => {
            return Err((
                4,
                local_err_doc(
                    "XLS_BINARY_UNSUPPORTED",
                    ErrorCategory::Unsupported,
                    ErrorPhase::Validate,
                    "capability drop esplicita: il contenitore binario BIFF .xls non e' supportato; usare .xlsx",
                ),
            ))
        }
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
                );
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
            "--max-input-entries" => {
                cli.limits.max_input_entries = parse_u64(it.next(), "--max-input-entries")?;
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
                .map_or_else(|| "Unresolved".to_owned(), |crs| format!("{:?}", crs.kind)),
        })
    });
    json!({
        "id": l.id.0,
        "name": l.name,
        "geometry": geom,
        "fields": fields,
    })
}

// Costruisce un `ResourceBudget` che riflette effettivamente i flag CLI
// `--max-rows`, `--max-columns`, `--max-output-bytes` e `--max-wkb-cell-bytes`
// / `--max-wkb-depth` (semantica per-cella).
//
// Storia del fix (finding #3 review 2026-08-15 + follow-up):
// 1. Prima del fix la CLI passava `ResourceBudget::default()` accanto ai
//    `Limits`, e i driver budget-driven ignoravano `--max-rows`/`--max-columns`.
// 2. Il primo fix li cablo' correttamente ma introdusse due regressioni:
//    (a) reader e writer condividevano lo stesso budget in `convert`, quindi
//        una riga contava due volte (R righe consumavano ~2R di quota `Rows`);
//    (b) `--max-wkb-components` (limite per singola cella WKB) veniva
//        trasformato nel contatore `GeometryComponents` cumulativo del budget,
//        che invece rappresenta il totale di componenti WKB su tutto il
//        dataset. Con il default 100_000 anche molte geometrie piccole
//        fallivano dopo 100k coordinate totali.
// 3. Questo fix separa i due ambiti: il helper produce un budget per
//    *singola operazione* (read o write); `cmd_convert` ne crea due copie
//    indipendenti; `GeometryComponents` conserva il default di
//    `ResourceLimits` (cumulativo, non derivato dal per-cella).
//
// Nota di perimetro: la fusione completa di `Limits` e `ResourceLimits` e'
// il PR-2 della roadmap `1.1.0`.
// Produce esplicitamente i due budget della pipeline `convert`: uno per il
// reader e uno per il writer. Follow-up review 2026-08-15: rendere questo
// il punto unico da cui `cmd_convert` prende i budget congela la regola
// "una riga si conta una volta" in un helper che i test possono esercitare
// direttamente. Il test associato deve continuare a passare se e solo se
// i due budget hanno contatori indipendenti.
fn conversion_budgets_from_limits(
    limits: &Limits,
) -> Result<(ResourceBudget, ResourceBudget), PlenoraIoError> {
    let read = resource_budget_from_limits(limits)?;
    let write = resource_budget_from_limits(limits)?;
    Ok((read, write))
}

fn resource_budget_from_limits(limits: &Limits) -> Result<ResourceBudget, PlenoraIoError> {
    let defaults = ResourceLimits::default();
    let wkb = limits.effective_wkb();
    let resource_limits = ResourceLimits {
        // Il campo Limits e' `usize`, il campo ResourceLimits e' `u64`: sui
        // target supportati (64-bit) il cast e' esatto. La `try_from`
        // conserva comunque la fail-closed su architetture ipotetiche 128-bit.
        rows: u64::try_from(limits.max_rows)
            .map_err(|_| PlenoraIoError::LimitExceeded("--max-rows fuori intervallo".to_owned()))?,
        columns: u64::try_from(limits.max_columns).map_err(|_| {
            PlenoraIoError::LimitExceeded("--max-columns fuori intervallo".to_owned())
        })?,
        // `nesting_depth` in `ResourceLimits` e `max_depth` in `WkbLimits`
        // hanno la stessa semantica (profondita' massima di annidamento) e
        // valgono per singola geometria, non cumulativi.
        nesting_depth: u64::try_from(wkb.max_depth).map_err(|_| {
            PlenoraIoError::LimitExceeded("--max-wkb-depth fuori intervallo".to_owned())
        })?,
        cell_bytes: u64::try_from(wkb.max_cell_bytes).map_err(|_| {
            PlenoraIoError::LimitExceeded("--max-wkb-cell-bytes fuori intervallo".to_owned())
        })?,
        output_bytes: limits.max_output_bytes,
        // `GeometryComponents` e' cumulativo (totale sul dataset). NON deriva
        // dal per-cella `max_wkb_components`, che vive nei `Limits.wkb` e
        // viene applicato dai driver a singola geometria via `WkbLimits`.
        // Conservare il default evita che dataset di molte piccole geometrie
        // (le piu' comuni) esauriscano una quota pensata per il per-cella.
        geometry_components: defaults.geometry_components,
        // Quote non esposte dalla CLI: restano ai default di `ResourceLimits`.
        memory_bytes: defaults.memory_bytes,
        concurrent_operations: defaults.concurrent_operations,
        output_expansion_ratio: defaults.output_expansion_ratio,
        duration_ms: defaults.duration_ms,
        spill_bytes: defaults.spill_bytes,
        decompression_ratio: defaults.decompression_ratio,
    };
    ResourceBudget::new(resource_limits)
}

// Unisce due mappe di opzioni di formato preservando la precedenza
// direzionale: `direzionali` sovrascrive per chiave `comuni`, cosi' una
// stessa chiave passata come `--opt` puo' essere ridefinita da `--in-opt` o
// `--out-opt` senza dipendere dall'ordine sulla riga di comando.
fn opts_uniti(
    comuni: &BTreeMap<String, String>,
    direzionali: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut risultato = comuni.clone();
    for (chiave, valore) in direzionali {
        risultato.insert(chiave.clone(), valore.clone());
    }
    risultato
}

fn read_options(cli: &Cli) -> Result<ReadOptions, PlenoraIoError> {
    // Finding #3: il budget deve riflettere i flag CLI. Fallire chiuso qui
    // preserva la semantica fail-closed dichiarata dal componente: un flag
    // fuori intervallo non deve degradare silenziosamente a un default.
    let resource_budget = resource_budget_from_limits(&cli.limits)?;
    Ok(ReadOptions {
        assume_crs: cli.assume_crs.clone(),
        format_options: cli.opts.clone(),
        limits: cli.limits,
        resource_budget,
        cancellation: CancellationToken::default(),
    })
}

// --- comandi ----------------------------------------------------------------

fn catalog_document(filegdb_available: bool) -> Value {
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
    let drivers = registry
        .descriptors()
        .into_iter()
        .map(|descriptor| {
            let mut document = serde_json::to_value(descriptor).unwrap_or(Value::Null);
            let is_filegdb = descriptor.id == "filegdb";
            if let Some(fields) = document.as_object_mut() {
                fields.insert(
                    "available".to_owned(),
                    Value::Bool(!is_filegdb || filegdb_available),
                );
                fields.insert(
                    "required_feature".to_owned(),
                    if is_filegdb {
                        Value::String("gdal-backend".to_owned())
                    } else {
                        Value::Null
                    },
                );
            }
            document
        })
        .collect::<Vec<_>>();
    json!({
        "status": "ok",
        "protocol_version": 1,
        "contract": "plenora-io-catalog-v1",
        "determinism": "byte_for_byte",
        "drivers": drivers,
    })
}

// Firma uniforme con gli altri `cmd_*`: il dispatch in `run()` richiede
// `CliResult` anche dove il comando non può fallire. La superficie CLI è
// congelata dal contratto `release/cli-protocol-v1.json`.
#[allow(clippy::unnecessary_wraps)]
fn cmd_catalog() -> CliResult {
    Ok(catalog_document(driver_filegdb::runtime_available()))
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
    let ropts = read_options(cli).map_err(map_err)?;
    let ds = driver.open(Source::Path(path), &ropts).map_err(map_err)?;
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
    let ropts = read_options(cli).map_err(map_err)?;
    let ds = driver.open(Source::Path(path), &ropts).map_err(map_err)?;
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

fn read_request(layer_id: u32, scope: ReadScope) -> ReadRequest {
    ReadRequest {
        layer: plenora_io_model::contract::LayerId(layer_id),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope,
        batch_target: BatchTarget::default(),
        cancellation: CancellationToken::default(),
    }
}

fn cmd_read(cli: &Cli) -> CliResult {
    let (driver, path) = open_source(cli)?;
    let ropts = read_options(cli).map_err(map_err)?;
    let ds = driver.open(Source::Path(path), &ropts).map_err(map_err)?;
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
        .open_layer_reader(&read_request(
            layer_id,
            cli.limit.map_or(ReadScope::Complete, |limit| {
                ReadScope::AcceptedRows(limit as u64)
            }),
        ))
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

// La pipeline di conversione è operation-atomic: apertura, validazione fino a
// EOF, trasferimento batch e pubblicazione devono restare in un'unica sequenza
// leggibile, con i fallimenti nell'ordine esatto in cui la CLI li espone.
#[allow(clippy::too_many_lines)]
fn cmd_convert(cli: &Cli) -> CliResult {
    if cli.positionals.len() < 2 {
        return Err(usage_err("convert richiede <ingresso> <uscita>"));
    }
    let in_path = PathBuf::from(&cli.positionals[0]);
    let out_path = PathBuf::from(&cli.positionals[1]);
    let src = driver_for_path(&in_path)?;
    let dst = driver_for_path(&out_path)?;
    // Finding #3 (follow-up review 2026-08-15): reader e writer devono avere
    // budget INDIPENDENTI. Condividere lo stesso `ResourceBudget` fa
    // consumare la quota `Rows`/`OutputBytes`/`GeometryComponents` due
    // volte per la stessa riga, quindi una conversione di R righe
    // esaurirebbe un budget da R (una `--max-rows R` fallirebbe intorno a
    // R/2 righe effettive). Il helper e' esposto come punto unico cosi'
    // che il test lo eserciti direttamente: qualunque futura tentazione di
    // riusare un solo budget deve passare da qui.
    let (read_budget, write_budget) =
        conversion_budgets_from_limits(&cli.limits).map_err(map_err)?;

    let ropts = ReadOptions {
        assume_crs: cli.assume_crs.clone(),
        // Finding #11 della review 2026-08-15: `--opt` era accettato dal
        // parser ma non consumato da `convert`. Ora `--opt` fa da base comune
        // per ingresso e uscita; `--in-opt` (e `--out-opt`) sovrascrivono per
        // chiave la stessa opzione, come dichiarato dal README.
        format_options: opts_uniti(&cli.opts, &cli.in_opts),
        limits: cli.limits,
        resource_budget: read_budget,
        cancellation: CancellationToken::default(),
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
        // Finding #11: vedi commento speculare in `ropts`.
        format_options: opts_uniti(&cli.opts, &cli.out_opts),
        limits: cli.limits,
        resource_budget: write_budget,
        cancellation: CancellationToken::default(),
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
            .open_layer_reader(&read_request(l.id.0, ReadScope::Complete))
            .map_err(map_err)?;
        let (mut rows, mut batches) = (0usize, 0usize);
        let mut layer_batches = Vec::new();
        while let Some(batch) = reader.next_batch().map_err(map_err)? {
            rows = rows.checked_add(batch.num_rows()).ok_or_else(|| {
                map_err(PlenoraIoError::LimitExceeded(
                    "overflow nel conteggio righe CLI".to_owned(),
                ))
            })?;
            batches = batches.checked_add(1).ok_or_else(|| {
                map_err(PlenoraIoError::LimitExceeded(
                    "overflow nel conteggio batch CLI".to_owned(),
                ))
            })?;
            layer_batches.push(batch);
        }
        let input_total = u64::try_from(rows).map_err(|_| {
            map_err(PlenoraIoError::LimitExceeded(
                "cardinalita sorgente non rappresentabile".to_owned(),
            ))
        })?;
        let sink_layer =
            plenora_io_model::contract::LayerId(u32::try_from(sink_idx).map_err(|_| {
                map_err(PlenoraIoError::LimitExceeded(
                    "numero di layer sorgente non rappresentabile".to_owned(),
                ))
            })?);
        writer
            .declare_input_total(sink_layer, input_total)
            .map_err(map_err)?;
        for batch in layer_batches {
            writer.write_to_layer(sink_layer, &batch).map_err(map_err)?;
        }
        read_loss.merge(&reader.loss_report());
        total_rows = total_rows.checked_add(rows).ok_or_else(|| {
            map_err(PlenoraIoError::LimitExceeded(
                "overflow nel conteggio totale righe CLI".to_owned(),
            ))
        })?;
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
        Some("--version" | "-V") => Ok(json!({
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

// Impronta stabile e non invertibile del messaggio di un panico. Duplica per
// scelta l'FNV-1a a 64 bit di `plenora-io-core::driver::impronta_del_panico`
// invece di importarlo: l'implementazione core resta interna alla libreria
// (nota di perimetro in `driver.rs:277-281`) e l'algoritmo qui deve restare
// stabile fra versioni di Rust, quindi non usiamo `DefaultHasher`.
fn impronta_del_panico(messaggio: &str) -> String {
    let mut stato: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in messaggio.as_bytes() {
        stato ^= u64::from(*byte);
        stato = stato.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{stato:016x}")
}

// Estrae il messaggio da un payload di panico senza assumerne il tipo:
// `panic!` con formato produce `String`, quello letterale `&'static str`.
fn messaggio_del_panico(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&'static str>().map_or_else(
        || {
            payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "panico senza messaggio".to_owned())
        },
        |testo| (*testo).to_owned(),
    )
}

// Hook silenzioso per il binario CLI.
//
// Storia (finding #13 review 2026-08-15 + follow-up):
// 1. Il default hook di Rust scriveva su stderr il messaggio completo del
//    panico, che nel nostro caso puo' contenere payload derivati dal file
//    letto (per esempio `arrow-buffer` riporta `slice offset=...`). La
//    CLI promette redazione dei valori derivati dall'input.
// 2. Il primo fix installava un hook che scriveva su stderr
//    `[panic] impronta=... location=...`. La redazione era rispettata, ma
//    la CLI promette anche "un solo documento JSON su stderr per errore".
//    Nel caso di panico caught dalla barriera `leggendo_arrow`, l'hook
//    stampava una riga E la CLI stampava l'envelope JSON: due uscite.
// 3. Questo fix silenzia il hook. Il wrapping `catch_unwind` in `main`
//    intercetta i panici che sfuggono al `run()` e li converte in un
//    unico envelope `plenora-io-error-v1` su stderr. La correlabilita' e'
//    preservata via l'impronta calcolata dentro l'envelope; il caso
//    "panico caught dentro la libreria" resta invisibile perche' la
//    libreria stessa produce gia' l'envelope corretto tramite
//    `PlenoraIoError`.
fn installa_hook_silenzioso() {
    std::panic::set_hook(Box::new(|_| {}));
}

fn envelope_panico(payload: &(dyn std::any::Any + Send)) -> Value {
    let messaggio = messaggio_del_panico(payload);
    let impronta = impronta_del_panico(&messaggio);
    let error = json!({
        "category": ErrorCategory::Internal,
        "phase": ErrorPhase::Read,
        "remote_effect": RemoteEffect::None,
        "retry": RetryDisposition::Never,
        "code": "PANIC",
        // Il messaggio NON contiene il payload del panico: solo l'impronta
        // FNV-1a a 64 bit del messaggio e la conferma testuale che questo
        // e' un panico non catturato dalla barriera della libreria.
        "message": format!("panico non catturato (impronta {impronta})"),
    });
    json!({
        "status": "error",
        "protocol_version": 1,
        "contract": "plenora-io-error-v1",
        "error": error,
    })
}

fn main() {
    installa_hook_silenzioso();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(Ok(doc)) => println!("{doc}"),
        Ok(Err((exit, doc))) => {
            eprintln!("{doc}");
            std::process::exit(exit);
        }
        Err(payload) => {
            eprintln!("{}", envelope_panico(payload.as_ref()));
            // Exit code 2 riservato agli errori di runtime del binario,
            // distinto dagli errori tipizzati che scelgono il proprio exit.
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_budget_riflette_i_flag_cli() {
        // Finding #3: verifica che i flag CLI attraversino effettivamente il
        // budget usato dai driver, invece di essere sovrascritti dai default
        // di `ResourceLimits`.
        use plenora_io_model::ResourceKind;
        let limits = Limits {
            max_rows: 7,
            max_columns: 5,
            max_output_bytes: 1_024,
            ..Limits::default()
        };
        let budget = resource_budget_from_limits(&limits).unwrap();
        assert_eq!(budget.remaining(ResourceKind::Rows), 7);
        assert_eq!(budget.remaining(ResourceKind::Columns), 5);
        assert_eq!(budget.limits().output_bytes, 1_024);
        // Le quote non esposte dalla CLI restano ai default del modello.
        assert_eq!(
            budget.limits().memory_bytes,
            ResourceLimits::default().memory_bytes
        );
    }

    #[test]
    fn resource_budget_rifiuta_flag_a_zero() {
        // `ResourceLimits::validate` rifiuta i limiti pari a zero: il helper
        // propaga l'errore invece di degradare a un default silenzioso.
        let limits = Limits {
            max_rows: 0,
            ..Limits::default()
        };
        assert!(resource_budget_from_limits(&limits).is_err());
    }

    #[test]
    fn resource_budget_non_deriva_geometry_components_dal_wkb_per_cella() {
        // Follow-up review 2026-08-15: `--max-wkb-components` (per cella)
        // NON deve alimentare il contatore cumulativo `GeometryComponents`.
        // Dataset di molte geometrie piccole avrebbero altrimenti esaurito
        // la quota dopo 100k coordinate totali (default WKB per-cella).
        use plenora_io_model::ResourceKind;
        let mut limits = Limits::default();
        limits.wkb.max_components = 42;
        let budget = resource_budget_from_limits(&limits).unwrap();
        assert_eq!(
            budget.remaining(ResourceKind::GeometryComponents),
            ResourceLimits::default().geometry_components,
            "GeometryComponents cumulativo non deve seguire il per-cella"
        );
    }

    #[test]
    fn conversion_budgets_hanno_contatori_indipendenti() {
        // Follow-up review 2026-08-15: il test precedente costruiva due
        // budget separati "a mano" e sarebbe passato anche se
        // `cmd_convert` fosse regredito a un budget unico. Qui esercitiamo
        // direttamente il helper `conversion_budgets_from_limits`, che
        // `cmd_convert` ora e' obbligato a usare come punto unico: una
        // regressione che chiami due volte lo stesso `resource_budget_from_limits`
        // andrebbe bene, ma sostituire il helper con un solo budget rompe
        // il contratto verificato qui.
        use plenora_io_model::ResourceKind;
        let limits = Limits {
            max_rows: 100,
            ..Limits::default()
        };
        let (read_budget, write_budget) = conversion_budgets_from_limits(&limits).unwrap();
        // Il reader "consuma" 60 righe della sua quota.
        let read_lease = read_budget.try_lease(ResourceKind::Rows, 60).unwrap();
        read_lease.commit(60).unwrap();
        // Il writer deve ancora avere 100 righe intere.
        assert_eq!(write_budget.remaining(ResourceKind::Rows), 100);
        // E non condivide contatori con il read_budget.
        assert!(!read_budget.is_same_budget(&write_budget));
        // Simmetricamente, il writer consuma output_bytes: il reader non
        // deve vederne l'effetto.
        let write_lease = write_budget
            .try_lease(ResourceKind::OutputBytes, 1_024)
            .unwrap();
        write_lease.commit(1_024).unwrap();
        assert_eq!(
            read_budget.remaining(ResourceKind::OutputBytes),
            read_budget.limits().output_bytes
        );
    }

    #[test]
    fn opts_uniti_preserva_precedenza_direzionale() {
        // Finding #11 della review 2026-08-15: la precedenza deve essere
        // "direzionale sovrascrive comune" e non deve dipendere dall'ordine
        // sulla riga di comando. Il test blocca la regola in modo che una
        // regressione la rompa esplicitamente.
        let mut comuni = BTreeMap::new();
        comuni.insert("delim".to_owned(), ",".to_owned());
        comuni.insert("shared".to_owned(), "base".to_owned());
        let mut direzionali = BTreeMap::new();
        direzionali.insert("shared".to_owned(), "override".to_owned());
        direzionali.insert("only-out".to_owned(), "yes".to_owned());
        let uniti = opts_uniti(&comuni, &direzionali);
        assert_eq!(uniti.get("delim").map(String::as_str), Some(","));
        assert_eq!(uniti.get("shared").map(String::as_str), Some("override"));
        assert_eq!(uniti.get("only-out").map(String::as_str), Some("yes"));
        // Le mappe di ingresso restano invariate.
        assert_eq!(comuni.get("shared").map(String::as_str), Some("base"));
        assert_eq!(direzionali.get("delim"), None);
    }

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
        if let Some(required) = envelope["current_producer"]["required_driver_fields"].as_array() {
            for driver in document["drivers"].as_array().unwrap() {
                for field in required {
                    let field = field.as_str().unwrap();
                    assert!(
                        driver.get(field).is_some(),
                        "{name}: campo driver {field} assente"
                    );
                }
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

    fn materialize_multibatch_geometry_ipc(
        directory: &tempfile::TempDir,
        first_batch_valid: bool,
    ) -> PathBuf {
        use std::fs::File;
        use std::sync::Arc;

        use arrow_array::{BinaryArray, RecordBatch};
        use arrow_ipc::writer::FileWriter;
        use arrow_schema::{DataType, Field, Schema};
        use plenora_io_model::contract::{FieldId, GeometryColumnContract, GeometryType};
        use plenora_io_model::crs::CrsResolution;
        use plenora_io_model::geometry::{with_contract_version, with_geometry_contract_metadata};

        const VALID_POINT: &[u8] = &[
            1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        const INVALID_WKB: &[u8] = &[1, 1, 0];
        let path = directory.path().join(if first_batch_valid {
            "late-invalid.arrow"
        } else {
            "prefix-invalid.arrow"
        });
        let mut geometry =
            GeometryColumnContract::wkb_xy(FieldId(0), "geometry", CrsResolution::Missing, true);
        geometry.set_exact_geometry_types(vec![GeometryType::Point]);
        let field = with_geometry_contract_metadata(
            &Field::new("geometry", DataType::Binary, true),
            &geometry,
        );
        let schema = with_contract_version(Arc::new(Schema::new(vec![field])));
        let first_values = (0..12)
            .map(|index| {
                Some(if first_batch_valid || index != 1 {
                    VALID_POINT
                } else {
                    INVALID_WKB
                })
            })
            .collect::<Vec<_>>();
        let first = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(first_values))],
        )
        .unwrap();
        let tail = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![Some(INVALID_WKB)]))],
        )
        .unwrap();
        let mut writer = FileWriter::try_new(File::create(&path).unwrap(), &schema).unwrap();
        writer.write(&first).unwrap();
        writer.write(&tail).unwrap();
        writer.finish().unwrap();
        path
    }

    #[test]
    fn read_limit_stops_before_invalid_tail_but_convert_remains_complete() {
        let directory = tempfile::tempdir().unwrap();
        let input = materialize_multibatch_geometry_ipc(&directory, true);
        let cli = parse(&[
            input.to_string_lossy().into_owned(),
            "--limit".to_owned(),
            "10".to_owned(),
        ])
        .unwrap();
        let summary = cmd_read(&cli).unwrap();
        assert_eq!(summary["rows_read"], 12);
        assert_eq!(summary["batches"], 1);
        assert_eq!(summary["truncated"], true);

        let output = directory.path().join("must-not-publish.arrow");
        let convert = parse(&[
            input.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
        ])
        .unwrap();
        let error = cmd_convert(&convert).unwrap_err();
        assert_eq!(error.0, 2, "{}", error.1);
        assert_eq!(error.1["error"]["code"], "FORMAT_ERROR");
        assert_eq!(
            error.1["error"]["row_diagnostics"]["examples"][0]["source_index"],
            12
        );
        assert!(!output.exists());
    }

    #[test]
    fn zero_read_limit_keeps_the_frozen_summary_without_observing_tail() {
        let directory = tempfile::tempdir().unwrap();
        let input = materialize_multibatch_geometry_ipc(&directory, false);
        let cli = parse(&[
            input.to_string_lossy().into_owned(),
            "--limit".to_owned(),
            "0".to_owned(),
        ])
        .unwrap();

        let summary = cmd_read(&cli).unwrap();
        assert_eq!(summary["rows_read"], 0);
        assert_eq!(summary["batches"], 0);
        assert_eq!(summary["truncated"], true);
        assert_eq!(summary["contract"], "plenora-io-read-v1");
    }

    #[test]
    fn read_limit_rejects_invalid_rows_inside_the_observed_prefix() {
        let directory = tempfile::tempdir().unwrap();
        let input = materialize_multibatch_geometry_ipc(&directory, false);
        let cli = parse(&[
            input.to_string_lossy().into_owned(),
            "--limit".to_owned(),
            "10".to_owned(),
        ])
        .unwrap();
        let error = cmd_read(&cli).unwrap_err();
        let diagnostics = &error.1["error"]["row_diagnostics"];
        assert_eq!(diagnostics["completeness"], "partial");
        assert_eq!(diagnostics["examples"][0]["source_index"], 1);
        assert_eq!(
            diagnostics["knowledge_limits"][0],
            "read_scope_row_limit_reached"
        );
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
        assert!(document["error"].get("row_diagnostics").is_none());
    }

    #[test]
    fn output_exists_keeps_the_frozen_cli_exit_and_category() {
        let (exit, document) = map_err(PlenoraIoError::OutputExists(
            "existing.unsupported".to_owned(),
        ));
        assert_eq!(exit, 3);
        assert_eq!(document["error"]["code"], "OUTPUT_EXISTS");
        assert_eq!(document["error"]["category"], "conflict");
    }

    #[test]
    fn row_diagnostics_are_preserved_in_the_cli_error_envelope() {
        let cause = "shapefile.inner_ring_without_outer".to_owned();
        let diagnostics = plenora_io_model::RowDiagnostics {
            contract: plenora_io_model::ROW_DIAGNOSTICS_CONTRACT.to_owned(),
            scope: plenora_io_model::RowDiagnosticScope::Read,
            index_basis: plenora_io_model::ROW_DIAGNOSTICS_INDEX_BASIS.to_owned(),
            completeness: plenora_io_model::RowDiagnosticsCompleteness::Complete,
            knowledge_limits: None,
            observed_total: 1,
            total: Some(1),
            input_total: None,
            counts: std::collections::BTreeMap::from([(cause.clone(), 1)]),
            examples_limit: 1,
            examples_truncated: false,
            examples: vec![plenora_io_model::RowDiagnosticExample {
                source_index: 17,
                cause,
                column: None,
                key: None,
                write_state: None,
            }],
            diagnostic_state_counts: None,
            write_outcome: None,
        };
        let error = PlenoraIoError::format("shp", "riga Shapefile non valida")
            .with_row_diagnostics(diagnostics);

        let (exit, document) = map_err(error);

        assert_eq!(exit, 2);
        assert_eq!(document["status"], "error");
        assert_eq!(document["protocol_version"], 1);
        assert_eq!(document["contract"], "plenora-io-error-v1");
        assert_eq!(document["error"]["code"], "FORMAT_ERROR");
        assert_eq!(document["error"]["category"], "data_mapping");
        assert_eq!(document["error"]["phase"], "read");
        assert_eq!(document["error"]["remote_effect"], "none");
        assert_eq!(
            document["error"]["retry"],
            serde_json::json!({"kind": "never"})
        );
        assert_eq!(
            document["error"]["row_diagnostics"]["contract"],
            plenora_io_model::ROW_DIAGNOSTICS_CONTRACT
        );
        assert_eq!(
            document["error"]["row_diagnostics"]["examples"][0]["source_index"],
            17
        );
    }

    #[test]
    fn cancellation_has_dedicated_exit_and_preserves_axes() {
        let error = PlenoraIoError::cancelled(ErrorPhase::Read, false);

        let (exit, document) = map_err(error);

        assert_eq!(exit, 130);
        assert_eq!(document["status"], "error");
        assert_eq!(document["protocol_version"], 1);
        assert_eq!(document["contract"], "plenora-io-error-v1");
        assert_eq!(document["error"]["code"], "CANCELLED");
        assert_eq!(document["error"]["category"], "cancelled");
        assert_eq!(document["error"]["phase"], "read");
        assert_eq!(document["error"]["remote_effect"], "none");
        assert_eq!(
            document["error"]["retry"],
            serde_json::json!({"kind": "never"})
        );
        assert!(document["error"].get("row_diagnostics").is_none());
    }

    #[test]
    fn data_mapping_changes_exit_only_and_preserves_frozen_error_codes() {
        for (error, expected_code) in [
            (PlenoraIoError::format("shp", "formato"), "FORMAT_ERROR"),
            (PlenoraIoError::Wkb("wkb".to_owned()), "FORMAT_ERROR"),
            (
                PlenoraIoError::Json(serde_json::from_str::<Value>("{").unwrap_err()),
                "FORMAT_ERROR",
            ),
        ] {
            let (exit, document) = map_err(error);
            assert_eq!(exit, 2);
            assert_eq!(document["error"]["code"], expected_code);
            assert_eq!(document["error"]["category"], "data_mapping");
        }
    }

    #[test]
    fn deadline_is_timeout_not_caller_cancellation() {
        let (exit, document) = map_err(PlenoraIoError::cancelled(ErrorPhase::Read, true));
        assert_eq!(exit, 1);
        assert_eq!(document["error"]["code"], "FORMAT_ERROR");
        assert_eq!(document["error"]["category"], "timeout");
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
    fn convert_round_trips_declared_unresolved_srid_only_without_synthesis() {
        use std::collections::HashMap;
        use std::fs::File;
        use std::sync::Arc;

        use arrow_ipc::reader::FileReader;
        use arrow_ipc::writer::FileWriter;
        use arrow_schema::{DataType, Field, Schema};
        use plenora_io_model::geometry::{
            with_contract_version, ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION,
            PLENORA_AXIS_ORDER_KEY, PLENORA_CRS_DEFINITION_KEY, PLENORA_CRS_ID_KEY,
            PLENORA_CRS_RESOLUTION_KEY, PLENORA_DIMENSIONS_KEY, PLENORA_ENCODING_KEY,
            PLENORA_GEOMETRY_TYPES_KEY, PLENORA_SRID_KEY, PLENORA_TYPES_DECLARATION_KEY,
        };

        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("srid-only-input.arrow");
        let output = directory.path().join("srid-only-output.arrow");
        let field = Field::new("geom", DataType::Binary, true).with_metadata(HashMap::from([
            (
                ARROW_EXTENSION_NAME_KEY.to_owned(),
                GEOARROW_WKB_EXTENSION.to_owned(),
            ),
            (PLENORA_ENCODING_KEY.to_owned(), "wkb".to_owned()),
            (PLENORA_DIMENSIONS_KEY.to_owned(), "xy".to_owned()),
            (PLENORA_TYPES_DECLARATION_KEY.to_owned(), "exact".to_owned()),
            (PLENORA_GEOMETRY_TYPES_KEY.to_owned(), "point".to_owned()),
            (
                PLENORA_CRS_RESOLUTION_KEY.to_owned(),
                "declared_unresolved".to_owned(),
            ),
            (PLENORA_SRID_KEY.to_owned(), "4326".to_owned()),
        ]));
        let schema = with_contract_version(Arc::new(Schema::new(vec![field])));
        FileWriter::try_new(File::create(&input).unwrap(), &schema)
            .unwrap()
            .finish()
            .unwrap();

        let cli = parse(&[
            input.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
        ])
        .unwrap();
        let document = cmd_convert(&cli).unwrap();
        assert_eq!(document["status"], "ok");

        let output_schema = FileReader::try_new(File::open(output).unwrap(), None)
            .unwrap()
            .schema();
        let metadata = output_schema.field(0).metadata();
        assert_eq!(
            metadata.get(PLENORA_SRID_KEY).map(String::as_str),
            Some("4326")
        );
        for key in [
            PLENORA_CRS_ID_KEY,
            PLENORA_CRS_DEFINITION_KEY,
            PLENORA_AXIS_ORDER_KEY,
        ] {
            assert!(!metadata.contains_key(key), "chiave sintetizzata: {key}");
        }
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

    #[cfg(not(feature = "gdal-backend"))]
    #[test]
    fn default_catalog_marks_filegdb_unavailable_and_names_the_required_feature() {
        let document = cmd_catalog().unwrap();
        let filegdb = document["drivers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|driver| driver["id"] == "filegdb")
            .expect("FileGDB deve restare individuabile nel catalogo");

        assert_eq!(filegdb["available"], false);
        assert_eq!(filegdb["required_feature"], "gdal-backend");
    }

    #[test]
    fn catalog_fields_have_exact_types_and_semantics_for_every_driver() {
        let document = catalog_document(false);
        for driver in document["drivers"].as_array().unwrap() {
            assert!(
                driver["available"].is_boolean(),
                "{}: available",
                driver["id"]
            );
            if driver["id"] == "filegdb" {
                assert_eq!(driver["available"], false);
                assert_eq!(driver["required_feature"].as_str(), Some("gdal-backend"));
            } else {
                assert_eq!(driver["available"], true, "{}", driver["id"]);
                assert!(driver["required_feature"].is_null(), "{}", driver["id"]);
            }
        }
    }

    #[test]
    fn feature_on_catalog_fails_closed_when_runtime_probe_is_unavailable() {
        let document = catalog_document(false);
        let filegdb = document["drivers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|driver| driver["id"] == "filegdb")
            .unwrap();
        assert!(!filegdb["available"].as_bool().unwrap());
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

    #[test]
    fn legacy_xls_extension_reports_the_explicit_capability_drop() {
        let Err(error) = driver_for_path(Path::new("legacy.xls")) else {
            panic!(".xls non deve essere instradato")
        };
        assert_eq!(error.0, 4);
        assert_eq!(error.1["error"]["code"], "XLS_BINARY_UNSUPPORTED");
        assert!(error.1["error"]["message"]
            .as_str()
            .unwrap()
            .contains("BIFF .xls"));
    }
}

#[cfg(test)]
mod conformance_tests;
