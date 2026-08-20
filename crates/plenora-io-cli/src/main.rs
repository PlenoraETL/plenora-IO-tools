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
use plenora_io_model::budget::{PipelineBudget, PipelineLimits};
use plenora_io_model::contract::{DataContract, LayerContract};
use plenora_io_model::geometry::is_geometry_field;
use plenora_io_model::{
    CancellationToken, ErrorCategory, ErrorPhase, NumeroStrutturale, PlenoraIoError, PublicMessage,
    RemoteEffect, RetryDisposition,
};

/// Errore CLI: (exit code, documento JSON d'errore).
type CliResult = Result<Value, (i32, Value)>;

/// Le estensioni che la CLI riconosce. Vocabolario chiuso di letterali
/// nostri: puo' comparire in un messaggio pubblico.
const ESTENSIONI_AMMESSE: &str =
    "parquet, geojson, csv, gpkg, shp, kml, xlsx, xls, dxf, gdb, arrow";

/// I flag che la CLI riconosce. Stessa ragione.
const OPZIONI_AMMESSE: &str = "--assume-crs, --durable, --in-opt, --layer, --limit,      --max-columns, --max-input-bytes, --max-input-entries, --max-output-bytes,      --max-rows, --max-vertices, --max-wkb-cell-bytes, --max-wkb-components,      --max-wkb-depth, --memory-bytes, --opt, --out-opt, --version";

#[allow(clippy::cast_possible_truncation)]
const fn saturating_u64(value: usize) -> u64 {
    if usize::BITS > u64::BITS && value > u64::MAX as usize {
        u64::MAX
    } else {
        value as u64
    }
}

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
    message: &PublicMessage,
) -> Value {
    err_doc(
        code,
        // `redatto` con `Generic`, non un costruttore di famiglia: il sito
        // usava `PlenoraIoError::new`, che imposta `code = Generic`. Un
        // costruttore di famiglia imporrebbe il proprio, cambiando il
        // quartetto — e' la regressione della tranche 2.
        &PlenoraIoError::redatto(
            plenora_io_model::IoErrorCode::Generic,
            category,
            phase,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        ),
    )
}

fn usage_err(message: &PublicMessage) -> (i32, Value) {
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
                    &PublicMessage::Curated(
                        "capability drop esplicita: il contenitore binario BIFF .xls                          non e' supportato; usare .xlsx",
                    ),
                ),
            ))
        }
        "dxf" => Box::new(driver_dxf::DxfDriver),
        "gdb" => Box::new(driver_filegdb::FileGdbDriver),
        "arrow" => Box::new(driver_ipc::IpcDriver),
        // Legato a `_`: l'estensione non serve piu' a nessuno qui, ed e' la
        // prova — in forma di binding — che non entra nel messaggio.
        _ => {
            return Err((
                4,
                local_err_doc(
                    "UNSUPPORTED",
                    ErrorCategory::Unsupported,
                    ErrorPhase::Validate,
                    &PublicMessage::CuratedPair(
                        "estensione non riconosciuta; ammesse:",
                        ESTENSIONI_AMMESSE,
                    ),
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
    /// I flag di quota, gia' nel tipo del modello unificato.
    ///
    /// Fino a S4.d atterravano in un `Limits` legacy e venivano tradotti piu'
    /// tardi. Il tipo intermedio non serviva a nulla se non a tenere in vita
    /// il modello vecchio nel punto piu' visibile del componente.
    limits: PipelineLimits,
}

fn kv(s: &str) -> Result<(String, String), (i32, Value)> {
    s.split_once('=')
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        // Il valore non esce: viene da argv. Il flag che lo ha introdotto e'
        // gia' nel messaggio del chiamante.
        .ok_or_else(|| {
            usage_err(&PublicMessage::Curated(
                "opzione di formato non nel formato chiave=valore",
            ))
        })
}

// `flag` e' `&'static str`: tutti i chiamanti passano il nome di un nostro
// flag. Il vocabolario e' chiuso, quindi il nome resta nel messaggio senza
// violare INV-10 — non e' testo runtime, e' uno dei nostri letterali.
fn parse_usize(value: Option<&String>, flag: &'static str) -> Result<usize, (i32, Value)> {
    value
        .ok_or_else(|| usage_err(&PublicMessage::CuratedPair(flag, "richiede un valore")))?
        .parse()
        .map_err(|_| {
            usage_err(&PublicMessage::CuratedPair(
                flag,
                "richiede un intero non negativo",
            ))
        })
}

// `flag` e' `&'static str`: tutti i chiamanti passano il nome di un nostro
// flag. Il vocabolario e' chiuso, quindi il nome resta nel messaggio senza
// violare INV-10 — non e' testo runtime, e' uno dei nostri letterali.
fn parse_u64(value: Option<&String>, flag: &'static str) -> Result<u64, (i32, Value)> {
    value
        .ok_or_else(|| usage_err(&PublicMessage::CuratedPair(flag, "richiede un valore")))?
        .parse()
        .map_err(|_| {
            usage_err(&PublicMessage::CuratedPair(
                flag,
                "richiede un intero non negativo",
            ))
        })
}

/// I flag che governano una quota, tutti insieme.
///
/// Estratti da `parse` perche' la funzione superava il tetto di righe, ma
/// stanno bene insieme anche di merito: sono l'unico gruppo di flag che
/// finisce nello stesso posto — `PipelineLimits` — e che condivide la stessa
/// disciplina fail-closed. Nessuno di loro degrada a un default quando il
/// valore e' assente o malformato.
///
/// Ritorna `None` se il flag non e' una quota, cosi' `parse` prosegue con i
/// propri casi invece di dover sapere quali sono.
///
/// # Errors
///
/// Se il flag e' una quota ma il valore manca o non e' un intero.
fn limite_da_flag<'a>(
    flag: &str,
    limiti: PipelineLimits,
    it: &mut impl Iterator<Item = &'a String>,
) -> Result<Option<PipelineLimits>, (i32, Value)> {
    let aggiornati = match flag {
        "--max-input-bytes" => {
            limiti.with_max_input_bytes(parse_u64(it.next(), "--max-input-bytes")?)
        }
        // Quota di memoria, distinta da quella dell'ingresso: da FZ-0.2.1
        // il tetto su una pagina Parquet non compressa ne e' la meta'.
        // Zero e incoerenze le rifiuta il modello, non il parser; la
        // motivazione estesa sta nel README.
        "--memory-bytes" => limiti.with_memory_bytes(parse_u64(it.next(), "--memory-bytes")?),
        "--max-input-entries" => {
            limiti.with_max_input_entries(parse_u64(it.next(), "--max-input-entries")?)
        }
        "--max-output-bytes" => {
            limiti.with_max_output_bytes(parse_u64(it.next(), "--max-output-bytes")?)
        }
        "--max-rows" => limiti.with_max_rows(parse_u64(it.next(), "--max-rows")?),
        "--max-columns" => limiti.with_max_columns(parse_u64(it.next(), "--max-columns")?),
        "--max-vertices" => limiti.with_max_vertices(parse_usize(it.next(), "--max-vertices")?),
        "--max-wkb-cell-bytes" => {
            limiti.with_max_wkb_cell_bytes(parse_usize(it.next(), "--max-wkb-cell-bytes")?)
        }
        "--max-wkb-components" => {
            limiti.with_max_wkb_components(parse_usize(it.next(), "--max-wkb-components")?)
        }
        "--max-wkb-depth" => limiti.with_max_wkb_depth(parse_usize(it.next(), "--max-wkb-depth")?),
        _ => return Ok(None),
    };
    Ok(Some(aggiornati))
}

fn parse(args: &[String]) -> Result<Cli, (i32, Value)> {
    let mut cli = Cli::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        // Le quote si riconoscono prima: sono un gruppo omogeneo che finisce
        // tutto in `PipelineLimits`, e tenerle qui allungava `parse` senza
        // aggiungere niente a chi la legge.
        if let Some(aggiornati) = limite_da_flag(a.as_str(), cli.limits, &mut it)? {
            cli.limits = aggiornati;
            continue;
        }
        match a.as_str() {
            "--assume-crs" => {
                cli.assume_crs = Some(
                    it.next()
                        .ok_or_else(|| {
                            usage_err(&PublicMessage::Curated("--assume-crs richiede un valore"))
                        })?
                        .clone(),
                );
            }
            "--layer" => {
                let v = it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--layer richiede un valore"))
                })?;
                cli.layer = Some(v.parse().map_err(|_| {
                    usage_err(&PublicMessage::Curated("--layer richiede un intero"))
                })?);
            }
            "--limit" => {
                let v = it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--limit richiede un valore"))
                })?;
                cli.limit = Some(v.parse().map_err(|_| {
                    usage_err(&PublicMessage::Curated("--limit richiede un intero"))
                })?);
            }
            "--durable" => cli.durable = true,
            "--opt" => {
                let (k, v) = kv(it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--opt richiede chiave=valore"))
                })?)?;
                cli.opts.insert(k, v);
            }
            "--in-opt" => {
                let (k, v) = kv(it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--in-opt richiede chiave=valore"))
                })?)?;
                cli.in_opts.insert(k, v);
            }
            "--out-opt" => {
                let (k, v) = kv(it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--out-opt richiede chiave=valore"))
                })?)?;
                cli.out_opts.insert(k, v);
            }
            other if other.starts_with("--") => {
                // Il token non esce: viene da argv. Resta la condizione, e
                // l'uso completo e' nel messaggio del comando senza argomenti.
                return Err(usage_err(&PublicMessage::CuratedPair(
                    "opzione sconosciuta; ammesse:",
                    OPZIONI_AMMESSE,
                )));
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

/// Costruisce la pipeline di lettura dai flag della CLI.
///
/// # Errors
///
/// Un flag fuori intervallo, o limiti che non superano la validazione del
/// modello: in entrambi i casi si fallisce chiuso invece di degradare a un
/// default che l'utente non ha chiesto.
fn read_pipeline(cli: &Cli) -> Result<ReadOptions, PlenoraIoError> {
    let bundle = PipelineBudget::builder().limits(cli.limits).build()?;
    Ok(ReadOptions::from_read_parts(bundle.into_read_parts()))
}

/// Costruisce i due rami di una conversione dallo **stesso** context.
///
/// Fino a S4.d la CLI costruiva due `ResourceBudget` scollegati: risolveva il
/// finding #3 — una riga non deve consumare due volte la stessa quota — ma al
/// prezzo di due pipeline che non sapevano l'una dell'altra. Memoria e spill
/// erano contati due volte, e il writer non poteva vedere l'input osservato
/// dal reader, che e' cio' da cui `output_expansion_ratio` deriva il proprio
/// tetto (INV-6).
///
/// `ConvertBudgetParts` risolve entrambe le cose: contatori cumulativi
/// indipendenti fra i due rami, `PipelineContext` condiviso.
///
/// # Errors
///
/// Un flag fuori intervallo, o limiti che non superano la validazione del
/// modello.
fn convert_pipeline(cli: &Cli) -> Result<(ReadOptions, WriteOptions), PlenoraIoError> {
    let bundle = PipelineBudget::builder().limits(cli.limits).build()?;
    let (read, write) = bundle.into_convert_parts().into_parts();
    Ok((
        ReadOptions::from_read_parts(read),
        WriteOptions::from_write_parts(write),
    ))
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
    let mut opzioni = read_pipeline(cli)?.with_format_options(cli.opts.clone());
    opzioni.assume_crs.clone_from(&cli.assume_crs);
    Ok(opzioni)
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
            let is_filegdb = descriptor.id() == "filegdb";
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
            .ok_or_else(|| usage_err(&PublicMessage::Curated("manca il percorso del file")))?,
    );
    let driver = driver_for_path(&path)?;
    Ok((driver, path))
}

fn cmd_inspect(cli: &Cli) -> CliResult {
    let (driver, path) = open_source(cli)?;
    let ropts = read_options(cli).map_err(map_err)?;
    let ds = driver.open(Source::Path(path), ropts).map_err(map_err)?;
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
    let ds = driver.open(Source::Path(path), ropts).map_err(map_err)?;
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
        "format": driver.descriptor().id(),
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
    let ds = driver.open(Source::Path(path), ropts).map_err(map_err)?;
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
                    &PublicMessage::CuratedWith(
                        "layer inesistente all'indice",
                        NumeroStrutturale::Indice(u64::from(layer_id)),
                    ),
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
        "format": driver.descriptor().id(),
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
        return Err(usage_err(&PublicMessage::Curated(
            "convert richiede <ingresso> <uscita>",
        )));
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
    // I due rami escono dalle stesse parti, quindi condividono il context:
    // contatori indipendenti, memoria e spill contati una volta sola, e il
    // writer vede l'input osservato dal reader (INV-6).
    let (mut ropts, mut wopts) = convert_pipeline(cli).map_err(map_err)?;
    ropts.assume_crs.clone_from(&cli.assume_crs);
    // Finding #11 della review 2026-08-15: `--opt` era accettato dal parser
    // ma non consumato da `convert`. Ora `--opt` fa da base comune per
    // ingresso e uscita; `--in-opt` (e `--out-opt`) sovrascrivono per chiave
    // la stessa opzione, come dichiarato dal README.
    ropts.format_options = opts_uniti(&cli.opts, &cli.in_opts);
    let ds = src.open(Source::Path(in_path), ropts).map_err(map_err)?;
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
                    &PublicMessage::CuratedWith(
                        "layer inesistente all'indice",
                        NumeroStrutturale::Indice(u64::from(id)),
                    ),
                ),
            )
        })?],
        None => all,
    };
    // Multi-layer verso destinazione single-layer: vietato (fail-closed).
    if selected.len() > 1 && !dst.descriptor().multi_layer() {
        return Err((
            4,
            local_err_doc(
                "SINGLE_LAYER_SINK",
                ErrorCategory::InvalidPlan,
                ErrorPhase::Validate,
                &PublicMessage::CuratedWith(
                    "destinazione single-layer ma la sorgente dichiara piu' layer;                      usare --layer per sceglierne uno. Layer nella sorgente:",
                    NumeroStrutturale::Conteggio(saturating_u64(
                        selected.len(),
                    )),
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
    wopts.durable = cli.durable;
    // Finding #11: vedi commento speculare in `ropts`.
    wopts.format_options = opts_uniti(&cli.opts, &cli.out_opts);
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
                map_err(PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "overflow nel conteggio righe CLI",
                )))
            })?;
            batches = batches.checked_add(1).ok_or_else(|| {
                map_err(PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "overflow nel conteggio batch CLI",
                )))
            })?;
            layer_batches.push(batch);
        }
        let input_total = u64::try_from(rows).map_err(|_| {
            map_err(PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "cardinalita sorgente non rappresentabile",
            )))
        })?;
        let sink_layer =
            plenora_io_model::contract::LayerId(u32::try_from(sink_idx).map_err(|_| {
                map_err(PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "numero di layer sorgente non rappresentabile",
                )))
            })?);
        writer
            .declare_input_total(sink_layer, input_total)
            .map_err(map_err)?;
        for batch in layer_batches {
            writer.write_to_layer(sink_layer, &batch).map_err(map_err)?;
        }
        read_loss.merge(&reader.loss_report());
        total_rows = total_rows.checked_add(rows).ok_or_else(|| {
            map_err(PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "overflow nel conteggio totale righe CLI",
            )))
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
        "from": src.descriptor().id(),
        "to": dst.descriptor().id(),
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
        _ => Err(usage_err(&PublicMessage::Curated(
            "uso: plenora-io <catalog|inspect|layers|read|convert> [args] | --version",
        ))),
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

/// La matrice di handoff verso `plenora-error-v1`, costruita dai tipi.
///
/// Serve a chi mantiene `plenora-contracts` per riallineare **senza leggere il
/// nostro codice**: elenca i campi che `plenora-io-error-v1` emette davvero, i
/// vincoli che valgono su di essi, e dove ciascuno va a finire nel contratto
/// successivo.
///
/// È generata dai tipi e non scritta a mano: un elenco copiato diverge alla
/// prima variante aggiunta, e diverge in silenzio — che è esattamente ciò che
/// una matrice di handoff non può fare.
///
/// **Non dichiara conformità a `plenora-contracts-next`.** Dice dove i campi
/// andranno, non che ci siano già: l'adozione è uno step breaking separato,
/// insieme alla CLI v2, agli exit code e alle capabilities.
#[cfg(test)]
fn matrice_di_handoff() -> Value {
    use plenora_io_model::IoErrorCode;

    // I codici sono enumerati dal tipo, non ricopiati: un `IoErrorCode` nuovo
    // compare qui senza che nessuno se ne ricordi.
    let codici: Vec<String> = IoErrorCode::TUTTI
        .iter()
        .map(|codice| {
            serde_json::to_value(codice)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("{codice:?}"))
        })
        .collect();

    json!({
        "contract": "plenora-io-handoff-v1",
        "generato_da": "plenora-io-cli, test la_matrice_di_handoff_e_aggiornata",
        "sorgente": {
            "contract": "plenora-io-error-v1",
            "protocol_version": 1,
            "stato": "invariato da S9: struttura, ordine e tipi identici alla baseline"
        },
        "destinazione": {
            "contract": "plenora-error-v1",
            "stato": "mappatura preparata, conformita' NON dichiarata",
            "nota": "l'adozione e' uno step breaking separato dopo S9, insieme a CLI v2, exit code e capabilities"
        },
        "vincoli": {
            // Il nome dice **cosa** e' misurato. `message_max_bytes` da solo si
            // legge come «byte sul wire», che e' falso: l'escaping JSON espande
            // virgolette e controlli, e nessuna misura avviene dopo la
            // serializzazione.
            "message_max_bytes_valore_decodificato": plenora_io_model::MAX_MESSAGE_BYTES,
            "message_max_bytes_serializzato": null,
            "message_max_bytes_serializzato_nota":
                "non promesso: l'escaping JSON espande (una virgoletta -> 2 byte, un controllo -> 6); se servira', va dichiarato a parte e misurato dopo la serializzazione",
            "message_non_e_chiave_di_compatibilita": true,
            "message_testo_runtime": "vietato, salvo il token bounded di un'opzione rifiutata prodotto dal validatore centrale",
            "assi_stabili": ["category", "phase", "code", "retry"]
        },
        "campi": [
            {
                "v1": "error.category", "next": "category",
                "nota": "asse stabile: e' con questi quattro che si correlano gli errori"
            },
            {
                "v1": "error.phase", "next": "phase",
                "nota": "asse stabile"
            },
            {
                "v1": "error.code", "next": "code",
                "nota": "asse stabile; vocabolario chiuso, sotto `vocabolari.code`"
            },
            {
                "v1": "error.retry", "next": "retry",
                "nota": "asse stabile"
            },
            {
                "v1": "error.remote_effect", "next": "remote_effect",
                "nota": "invariato"
            },
            {
                "v1": "error.message", "next": "message",
                "nota": "testo curato, deterministico, <= 2048 byte. **Non** e' un identificatore: il testo cambia con S9 e cambiera' ancora"
            },
            {
                "v1": "error.row_diagnostics", "next": "details.row_diagnostics",
                "nota": "scende di un livello; contratto interno invariato"
            },
            {
                "v1": null, "next": "provider",
                "da": "PlenoraIoError::driver",
                "stato": "da decidere",
                "nota": "il driver esiste nel tipo Rust ma **non e' emesso** da v1: e' un campo nuovo per la destinazione, non una rinomina"
            },
            {
                "v1": null, "next": "details",
                "da": "PlenoraIoError::field, capability_reason",
                "nota": "il contesto strutturato non e' emesso da v1; confluisce in `details` nella destinazione"
            }
        ],
        "vocabolari": {
            "code": codici
        },
        "domande_aperte": [
            {
                "id": "driver-e-un-provider",
                "campo": "provider",
                "domanda": "un driver di formato IO e' davvero un `provider`, oppure appartiene a un `details` component-owned come `format_id`?",
                "contesto": "«provider» suggerisce un servizio o un backend remoto; qui e' il formato del file — csv, geoparquet, shapefile — scelto dal chiamante e senza effetto remoto. Se la destinazione intende `provider` nel primo senso, `details.format_id` descrive meglio cio' che il valore e'.",
                "bloccante_per_s9": false,
                "nota": "S9 prosegue senza attendere: la mappatura e' preparata, la conformita' non e' dichiarata, e il DTO e' l unico punto che dovra' cambiare quando la risposta arrivera'."
            }
        ]
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
    use plenora_io_model::budget::OperationCounter;

    /// Opzioni di scrittura sul modello unificato, per i test.
    fn opzioni_scrittura_di_prova() -> plenora_io_core::WriteOptions {
        match PipelineBudget::builder().build() {
            Ok(bundle) => {
                plenora_io_core::WriteOptions::from_write_parts(bundle.into_write_parts())
            }
            Err(error) => unreachable!("bundle di test: {error:?}"),
        }
    }

    #[test]
    fn i_flag_atterrano_nei_limiti_della_pipeline() {
        let cli = parse(&[
            "convert".to_owned(),
            "--max-rows".to_owned(),
            "7".to_owned(),
            "--max-columns".to_owned(),
            "5".to_owned(),
            "--max-output-bytes".to_owned(),
            "1024".to_owned(),
        ])
        .expect("flag validi");

        assert_eq!(cli.limits.max_rows(), 7);
        assert_eq!(cli.limits.max_columns(), 5);
        assert_eq!(cli.limits.max_output_bytes(), 1_024);
        // Le quote non esposte dalla CLI restano ai default del modello.
        assert_eq!(
            cli.limits.memory_bytes(),
            PipelineLimits::default().memory_bytes()
        );
    }

    #[test]
    fn la_pipeline_rifiuta_flag_a_zero() {
        // `PipelineLimits::validate` rifiuta le quote nulle: la costruzione
        // propaga l'errore invece di degradare a un default silenzioso.
        let cli = parse(&[
            "convert".to_owned(),
            "--max-rows".to_owned(),
            "0".to_owned(),
        ])
        .expect("il parser accetta lo zero, e' il modello a rifiutarlo");
        assert!(PipelineBudget::builder()
            .limits(cli.limits)
            .build()
            .is_err());
    }

    #[test]
    fn memory_bytes_ha_un_default_e_un_flag_che_lo_cambia() {
        // Il default non si muove: chi non passa il flag ottiene ciò che
        // otteneva prima.
        let senza = parse(&["read".to_owned(), "x".to_owned()]).expect("flag validi");
        assert_eq!(
            senza.limits.memory_bytes(),
            PipelineLimits::default().memory_bytes()
        );

        let con = parse(&[
            "read".to_owned(),
            "x".to_owned(),
            "--memory-bytes".to_owned(),
            "134217728".to_owned(),
        ])
        .expect("flag validi");
        assert_eq!(con.limits.memory_bytes(), 134_217_728);
        // La memoria non tocca le altre quote: sono distinte apposta.
        assert_eq!(
            con.limits.max_input_bytes(),
            PipelineLimits::default().max_input_bytes()
        );
    }

    #[test]
    fn memory_bytes_rifiuta_zero_e_valori_non_rappresentabili() {
        // Non rappresentabile: il parser si ferma prima del modello.
        for grezzo in ["0x10", "-1", "1.5", "18446744073709551616", ""] {
            assert!(
                parse(&[
                    "read".to_owned(),
                    "x".to_owned(),
                    "--memory-bytes".to_owned(),
                    grezzo.to_owned(),
                ])
                .is_err(),
                "'{grezzo}' doveva essere rifiutato dal parser"
            );
        }
        // Valore mancante.
        assert!(parse(&[
            "read".to_owned(),
            "x".to_owned(),
            "--memory-bytes".to_owned(),
        ])
        .is_err());

        // Zero: il parser lo accetta come intero, il **modello** lo rifiuta.
        // La divisione dei compiti è voluta — il parser sa cos'è un numero, il
        // modello sa quali numeri hanno senso insieme.
        let cli = parse(&[
            "read".to_owned(),
            "x".to_owned(),
            "--memory-bytes".to_owned(),
            "0".to_owned(),
        ])
        .expect("il parser accetta lo zero");
        assert!(PipelineBudget::builder()
            .limits(cli.limits)
            .build()
            .is_err());

        // Sotto `max_wkb_cell_bytes` senza abbassarla: rifiutato, perché una
        // cella non può valere più di tutta la memoria.
        let cli = parse(&[
            "read".to_owned(),
            "x".to_owned(),
            "--memory-bytes".to_owned(),
            "1024".to_owned(),
        ])
        .expect("il parser accetta il valore");
        assert!(PipelineBudget::builder()
            .limits(cli.limits)
            .build()
            .is_err());
    }

    #[test]
    fn memory_bytes_arriva_a_lettura_e_scrittura_dallo_stesso_context() {
        let cli = parse(&[
            "convert".to_owned(),
            "a".to_owned(),
            "b".to_owned(),
            "--memory-bytes".to_owned(),
            "134217728".to_owned(),
        ])
        .expect("flag validi");
        let (ropts, wopts) = convert_pipeline(&cli).expect("pipeline valida");

        assert_eq!(
            ropts.budget().context().limits().memory_bytes(),
            134_217_728
        );
        assert_eq!(
            wopts.budget().context().limits().memory_bytes(),
            134_217_728
        );
        // Non due context uguali: **lo stesso**. È la proprietà che S4.d aveva
        // stabilito e che un flag nuovo potrebbe rompere passando da una strada
        // laterale.
        assert!(ropts
            .budget()
            .context()
            .is_same_pipeline(wopts.budget().context()));

        // E la lettura semplice lo riceve dallo stesso posto.
        let solo_lettura = read_pipeline(&cli).expect("pipeline valida");
        assert_eq!(
            solo_lettura.budget().context().limits().memory_bytes(),
            134_217_728
        );
    }

    /// Un `GeoParquet` con molte righe e celle minuscole.
    ///
    /// La forma conta: la pagina deve essere grande **senza** che nessuna
    /// singola cella lo sia, altrimenti abbassando la memoria scatterebbe
    /// `max_wkb_cell_bytes` e il test misurerebbe un altro controllo.
    fn scrivi_molte_geometrie_piccole(path: &std::path::Path) {
        use std::collections::HashMap;
        use std::sync::Arc;

        use arrow_array::{BinaryArray, RecordBatch};
        use arrow_schema::{DataType, Field, Schema, SchemaRef};
        use plenora_io_core::{FormatDriver, Sink, WriteLayer, WritePlan};
        use plenora_io_model::contract::{
            CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
        };
        use plenora_io_model::crs::{CrsKind, ResolvedCrs};
        use plenora_io_model::geometry::{
            ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, GEO_CRS_KEY,
        };
        use plenora_io_model::wkb::{encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};

        let schema: SchemaRef = Arc::new(Schema::new(vec![Field::new(
            "geometry",
            DataType::Binary,
            true,
        )
        .with_metadata(HashMap::from([
            (
                ARROW_EXTENSION_NAME_KEY.to_owned(),
                GEOARROW_WKB_EXTENSION.to_owned(),
            ),
            (GEO_CRS_KEY.to_owned(), "EPSG:4326".to_owned()),
        ]))]));
        let celle: Vec<Vec<u8>> = (0..50_000)
            .map(|i| {
                let x = f64::from(i);
                encode_wkb(
                    &WkbGeometry {
                        value: WkbValue::Point(WkbCoordinate {
                            x,
                            y: x,
                            z: None,
                            m: None,
                        }),
                        dimensions: CoordinateDimensions::Xy,
                        srid: None,
                    },
                    WkbFlavor::Iso,
                )
                .unwrap()
            })
            .collect();
        let colonna =
            BinaryArray::from(celle.iter().map(|c| Some(c.as_slice())).collect::<Vec<_>>());
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(colonna)]).unwrap();

        let mut geometria = GeometryColumnContract::wkb_passthrough(
            FieldId(0),
            "geometry",
            ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
            true,
        );
        geometria.set_exact_geometry_types(vec![GeometryType::Point]);
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometria),
                },
            }],
        };
        let driver = driver_geoparquet::GeoParquetDriver;
        let mut writer = driver
            .create(
                Sink::Path(path.to_path_buf()),
                &plan,
                &convert_pipeline(&Cli::default()).unwrap().1,
            )
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
    }

    /// Abbassare `--memory-bytes` abbassa il tetto sulla pagina `GeoParquet`.
    ///
    /// È la ragione per cui il flag esiste: senza, dentro un container con meno
    /// memoria del predefinito il tetto restava a 256 MiB — cioè proprio dove
    /// andrebbe stretto non si poteva stringerlo.
    ///
    /// Il file ha molte righe con celle minuscole, non una cella grande: così
    /// la pagina è grande senza che `--max-wkb-cell-bytes` c'entri, e il rifiuto
    /// che si osserva è quello sotto esame e non un altro.
    #[test]
    fn abbassare_memory_bytes_abbassa_il_tetto_della_pagina_geoparquet() {
        use plenora_io_core::{
            BatchTarget, FormatDriver, ProjectionMode, ReadRequest, ReadScope, Source,
        };
        use plenora_io_model::contract::LayerId;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pagine.parquet");
        scrivi_molte_geometrie_piccole(&path);

        let richiesta = ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
            cancellation: plenora_io_model::CancellationToken::default(),
        };
        let driver = driver_geoparquet::GeoParquetDriver;
        let leggi = |flag: &[&str]| -> Result<(), plenora_io_model::PlenoraIoError> {
            let mut argomenti = vec!["read".to_owned(), path.display().to_string()];
            argomenti.extend(flag.iter().map(|f| (*f).to_owned()));
            let cli = parse(&argomenti).expect("flag validi");
            let opzioni = read_pipeline(&cli).expect("pipeline valida");
            let dataset = driver
                .open(Source::Path(path.clone()), opzioni)
                .expect("il file sta sotto le quote di ingresso");
            match dataset.open_layer_reader(&richiesta) {
                Err(errore) => Err(errore),
                Ok(mut lettore) => lettore.next_batch().map(|_| ()),
            }
        };

        // Con il predefinito il tetto è 256 MiB e la pagina passa.
        leggi(&[]).expect("con la memoria predefinita il file si legge");

        // Abbassando la memoria il tetto scende sotto la pagina. Le celle sono
        // di ventuno byte, quindi `--max-wkb-cell-bytes` non è il vincolo che
        // scatta: è la memoria.
        let errore = leggi(&["--memory-bytes", "2000000", "--max-wkb-cell-bytes", "1000"])
            .expect_err("con meno memoria il tetto scende sotto la pagina");
        assert_eq!(
            errore.message,
            "pagina Parquet che dichiara piu' byte non compressi della memoria disponibile",
            "{errore}"
        );
    }

    #[test]
    fn geometry_components_non_deriva_dal_wkb_per_cella() {
        // Follow-up review 2026-08-15: `--max-wkb-components` (per cella) NON
        // deve alimentare il contatore cumulativo `GeometryComponents`.
        // Dataset di molte geometrie piccole avrebbero altrimenti esaurito la
        // quota dopo 100k coordinate totali.
        let cli = parse(&[
            "convert".to_owned(),
            "--max-wkb-components".to_owned(),
            "42".to_owned(),
        ])
        .expect("flag validi");

        assert_eq!(
            cli.limits.max_wkb_components(),
            42,
            "il per-cella segue il flag"
        );
        assert_eq!(
            cli.limits.max_geometry_components(),
            PipelineLimits::default().max_geometry_components(),
            "il cumulativo non deve seguire il per-cella"
        );
    }

    #[test]
    fn i_due_rami_di_convert_hanno_contatori_indipendenti_e_context_condiviso() {
        // Il finding #3 chiedeva contatori indipendenti: una riga non deve
        // consumare due volte la stessa quota. Fino a S4.d la CLI lo otteneva
        // con due budget **scollegati**, che pero' contavano due volte anche
        // memoria e spill e impedivano al writer di vedere l'input osservato
        // dal reader (INV-6). Ora i due rami escono dalle stesse parti.
        let cli = Cli {
            limits: PipelineLimits::default().with_max_rows(100),
            ..Cli::default()
        };
        let (ropts, wopts) = convert_pipeline(&cli).expect("pipeline valida");

        ropts
            .budget()
            .try_lease(OperationCounter::Rows, 60)
            .expect("lease")
            .commit(60)
            .expect("commit");
        assert_eq!(wopts.budget().remaining(OperationCounter::Rows), 100);
        assert!(!ropts.budget().shares_counters_with(wopts.budget()));

        // Context condiviso: memoria, spill e deadline sono gli stessi.
        assert!(ropts
            .budget()
            .context()
            .is_same_pipeline(wopts.budget().context()));
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
            .create(
                Sink::Path(path.clone()),
                &plan,
                &opzioni_scrittura_di_prova(),
            )
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
        assert_eq!(cli.limits.max_rows(), 123);
        assert_eq!(cli.limits.max_wkb_depth(), 9);
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
        let (exit, document) = map_err(PlenoraIoError::destinazione_esistente());
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
        let error = PlenoraIoError::formato_redatto(
            "shp",
            &PublicMessage::Curated("riga Shapefile non valida"),
        )
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

    /// `plenora-io-error-v1` ha esattamente questi campi, e non ne acquista
    /// altri per sbaglio.
    ///
    /// S9 ha riempito `PlenoraIoError::driver` e `PlenoraIoError::field` su
    /// molti piu' errori di prima — `field` con un `ContractIdentifier`, che e'
    /// il punto della migrazione. Nessuno dei due e' emesso da questo
    /// envelope, e **non deve diventarlo per effetto collaterale**: aggiungere
    /// un campo al wire e' un cambiamento di contratto, non una conseguenza di
    /// un refactor interno.
    ///
    /// Il test guarda l'insieme delle chiavi, non le singole: un `assert` per
    /// campo assente si dimentica del campo che nessuno ha ancora inventato.
    #[test]
    fn il_wire_v1_ha_esattamente_i_campi_dichiarati_e_non_acquista_field() {
        use plenora_io_model::{ContractIdentifier, ErrorContext, PublicMessage};

        // Un errore con contesto ricco: driver, campo e ragione di capability.
        let schema = arrow_schema::Schema::new(vec![arrow_schema::Field::new(
            "geometry",
            arrow_schema::DataType::Binary,
            true,
        )]);
        let identificatore =
            ContractIdentifier::from_schema_field(&schema, plenora_io_model::contract::FieldId(0))
                .expect("il nome e' nominabile");
        let contesto = ErrorContext::nuovo()
            .con_driver("geoparquet")
            .con_identificatore(identificatore);
        let error = PlenoraIoError::schema_redatto(&PublicMessage::Curated(
            "campo esposto non presente nello schema fisico",
        ))
        .con_contesto(&contesto);

        // Il contesto e' arrivato nel tipo Rust...
        assert_eq!(error.driver.as_deref(), Some("geoparquet"));
        assert_eq!(error.field.as_deref(), Some("geometry"));

        // ...e non sul wire.
        let (_, document) = map_err(error);
        let campi: std::collections::BTreeSet<&str> = document["error"]
            .as_object()
            .expect("l'errore e' un oggetto")
            .keys()
            .map(String::as_str)
            .collect();
        let attesi: std::collections::BTreeSet<&str> = [
            "category",
            "phase",
            "remote_effect",
            "retry",
            "code",
            "message",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            campi, attesi,
            "plenora-io-error-v1 ha cambiato forma: {campi:?}"
        );

        let messaggio = document["error"]["message"]
            .as_str()
            .expect("il messaggio e' una stringa");
        assert!(
            !messaggio.contains("geometry"),
            "il nome del campo non deve rientrare dal messaggio: {messaggio}"
        );
    }

    /// La busta degli errori d'uso della CLI, che e' una **via diversa** da
    /// `map_err`: passa per `usage_err` -> `local_err_doc` -> `err_doc`, e fino
    /// alla tranche 14 nessun test ne verificava la forma.
    #[test]
    fn la_busta_degli_errori_d_uso_ha_esattamente_le_sei_chiavi_v1() {
        let (exit, documento) = usage_err(&PublicMessage::CuratedPair(
            "opzione sconosciuta; ammesse:",
            OPZIONI_AMMESSE,
        ));

        assert_eq!(exit, 2, "l'exit degli errori d'uso e' 2");
        assert_eq!(documento["protocol_version"], 1);
        assert_eq!(documento["contract"], "plenora-io-error-v1");

        let campi: std::collections::BTreeSet<&str> = documento["error"]
            .as_object()
            .expect("l'errore e' un oggetto")
            .keys()
            .map(String::as_str)
            .collect();
        let attesi: std::collections::BTreeSet<&str> = [
            "category",
            "phase",
            "remote_effect",
            "retry",
            "code",
            "message",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            campi, attesi,
            "plenora-io-error-v1 ha cambiato forma sulla via d'uso: {campi:?}"
        );

        // Il quartetto della via d'uso, che nessuno snapshot puo' vedere: il
        // `code` sul wire non viene da `IoErrorCode` ma dal letterale passato a
        // `err_doc`, quindi va fissato qui o non e' fissato da nessuna parte.
        let errore = &documento["error"];
        assert_eq!(errore["category"], "invalid_configuration");
        assert_eq!(errore["phase"], "validate");
        assert_eq!(errore["remote_effect"], "none");
        // `retry` sul wire e' un oggetto `{"kind": …}`, non una stringa nuda:
        // fissato sull'osservato, non su come me lo immaginavo.
        assert_eq!(errore["retry"]["kind"], "never");
        assert_eq!(errore["code"], "CLI_USAGE");
    }

    /// Nessun argomento della riga di comando finisce nella busta.
    ///
    /// I due siti che lo facevano — il token di un'opzione sconosciuta e il
    /// valore di `--opt` mal formato — passavano `argv` dentro `format!`. Il
    /// test costruisce gli argomenti con un marcatore improbabile e verifica
    /// che non compaia nel documento serializzato.
    #[test]
    fn nessun_argomento_della_riga_di_comando_entra_nella_busta() {
        const MARCATORE: &str = "zzMARCATORE-ARGVzz";

        let casi = vec![
            vec![format!("--{MARCATORE}")],
            vec!["--opt".to_owned(), MARCATORE.to_owned()],
            vec!["--in-opt".to_owned(), MARCATORE.to_owned()],
            vec!["--out-opt".to_owned(), MARCATORE.to_owned()],
            vec!["--layer".to_owned(), MARCATORE.to_owned()],
            vec!["--max-rows".to_owned(), MARCATORE.to_owned()],
        ];

        for argomenti in casi {
            let Err((_, documento)) = parse(&argomenti) else {
                panic!("{argomenti:?}: doveva essere rifiutato");
            };
            let testo = documento.to_string();
            assert!(
                !testo.contains(MARCATORE),
                "{argomenti:?}: l'argomento e' uscito nella busta: {testo}"
            );
        }
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
            (
                PlenoraIoError::formato_redatto("shp", &PublicMessage::Curated("formato")),
                "FORMAT_ERROR",
            ),
            (
                PlenoraIoError::wkb_redatto(&PublicMessage::Curated("wkb")),
                "FORMAT_ERROR",
            ),
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
        let error = PlenoraIoError::redatto(
            plenora_io_model::IoErrorCode::Generic,
            ErrorCategory::Transient,
            ErrorPhase::Connect,
            RemoteEffect::None,
            RetryDisposition::After(2_750),
            &PublicMessage::Curated("servizio temporaneamente non disponibile"),
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
        let (exit, document) = usage_err(&PublicMessage::Curated("argomento mancante"));
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
            .create(
                Sink::Path(input.clone()),
                &plan,
                &opzioni_scrittura_di_prova(),
            )
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
                match plenora_io_model::budget::PipelineBudget::builder().build() {
                    Ok(bundle) => {
                        plenora_io_core::ReadOptions::from_read_parts(bundle.into_read_parts())
                    }
                    Err(error) => unreachable!("bundle di test: {error:?}"),
                },
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
        let (_, document) = map_err(plenora_io_model::PlenoraIoError::crs_non_risolto_redatto(
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
                .id(),
            "geojson"
        );
        assert_eq!(
            driver_for_path(Path::new("x.gpkg"))
                .unwrap()
                .descriptor()
                .id(),
            "gpkg"
        );
        assert_eq!(
            driver_for_path(Path::new("x.parquet"))
                .unwrap()
                .descriptor()
                .id(),
            "geoparquet"
        );
        assert_eq!(
            driver_for_path(Path::new("x.shp.d"))
                .unwrap()
                .descriptor()
                .id(),
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

    /// Snapshot **del solo legacy**: `read_mode`, driver per driver.
    ///
    /// Sta da solo, separato da quello della tripla, perché prova una cosa
    /// diversa: che S8 **non abbia toccato** un campo che `plenora-io-catalog-v1`
    /// emette da sempre. Se i due snapshot fossero uno, una modifica al legacy
    /// mascherata da aggiornamento della tripla passerebbe in una diff sola.
    ///
    /// I valori sono quelli precedenti a S8, byte per byte. Non vanno
    /// riallineati a `native_read_mode`: la divergenza fra i due **è**
    /// l'informazione che lo split esiste per esporre.
    #[test]
    fn il_read_mode_legacy_e_preservato_driver_per_driver() {
        const ATTESI: &[(&str, &str)] = &[
            ("csv", "streaming_sequential"),
            ("dxf", "streaming_sequential"),
            ("filegdb", "materializing"),
            ("geojson", "streaming_sequential"),
            ("geoparquet", "streaming_columnar"),
            ("gpkg", "streaming_sequential"),
            ("ipc", "streaming_sequential"),
            ("kml", "streaming_sequential"),
            ("shp", "streaming_sequential"),
            ("xls", "streaming_sequential"),
        ];
        let document = catalog_document(false);
        let drivers = document["drivers"].as_array().unwrap();
        assert_eq!(drivers.len(), ATTESI.len(), "driver aggiunti o rimossi");
        for (id, atteso) in ATTESI {
            let driver = drivers
                .iter()
                .find(|driver| driver["id"] == *id)
                .unwrap_or_else(|| panic!("{id} assente dal catalogo"));
            assert_eq!(
                driver["read_mode"].as_str(),
                Some(*atteso),
                "{id}: il read_mode legacy e' cambiato"
            );
        }
    }

    /// Snapshot della **tripla dichiarativa** di INV-7.
    ///
    /// Le due colonne che non variano — `operation_atomic` e
    /// `adaptive_memory_then_disk` — non sono ridondanti: sono ciò che
    /// `BudgetedReader` impone a *tutti*, e un driver che ne dichiarasse altre
    /// starebbe descrivendo un comportamento che l'adapter non gli lascia
    /// avere. È il caso che questo snapshot prende.
    #[test]
    fn la_tripla_di_inv7_e_quella_dichiarata_da_ogni_driver() {
        const ATTESI: &[(&str, &str)] = &[
            ("csv", "streaming_sequential"),
            ("dxf", "materialized"),
            ("filegdb", "streaming_sequential"),
            ("geojson", "streaming_sequential"),
            ("geoparquet", "streaming_random"),
            ("gpkg", "streaming_random"),
            ("ipc", "streaming_random"),
            ("kml", "materialized"),
            ("shp", "streaming_sequential"),
            ("xls", "materialized"),
        ];
        let document = catalog_document(false);
        let drivers = document["drivers"].as_array().unwrap();
        assert_eq!(drivers.len(), ATTESI.len(), "driver aggiunti o rimossi");
        for (id, nativo) in ATTESI {
            let driver = drivers
                .iter()
                .find(|driver| driver["id"] == *id)
                .unwrap_or_else(|| panic!("{id} assente dal catalogo"));
            assert_eq!(
                driver["native_read_mode"].as_str(),
                Some(*nativo),
                "{id}: native_read_mode"
            );
            assert_eq!(
                driver["effective_delivery"].as_str(),
                Some("operation_atomic"),
                "{id}: l'adapter comune drena prima del primo batch, per tutti"
            );
            assert_eq!(
                driver["buffering"].as_str(),
                Some("adaptive_memory_then_disk"),
                "{id}: lo spool dell'adapter comune vale per tutti"
            );
        }
    }

    /// La tripla è **completa** e il legacy non è derivato da essa.
    ///
    /// Due proprietà in un test perché sono la stessa affermazione vista da due
    /// lati: i tre campi ci sono per ogni driver, e il quarto — il legacy —
    /// **non** si ricava dai primi tre. Se qualcuno un giorno derivasse
    /// `read_mode` da `native_read_mode`, i sette driver che oggi divergono
    /// tornerebbero a coincidere e il campo tornerebbe a non dire niente:
    /// esattamente il difetto L0.4 che INV-7 chiude.
    #[test]
    fn ogni_driver_dichiara_la_tripla_e_il_legacy_puo_divergere() {
        let document = catalog_document(false);
        let drivers = document["drivers"].as_array().unwrap();
        let mut divergenti = 0;
        for driver in drivers {
            let id = &driver["id"];
            for campo in ["native_read_mode", "effective_delivery", "buffering"] {
                assert!(
                    driver[campo].is_string(),
                    "{id}: {campo} assente o non stringa"
                );
            }
            // I due valori non sono lo stesso vocabolario — `materializing` non
            // è `materialized`, `streaming_columnar` non esiste fra i nativi —
            // quindi la divergenza si conta sui casi in cui *nemmeno*
            // l'intenzione coincide.
            let legacy = driver["read_mode"].as_str().unwrap();
            let nativo = driver["native_read_mode"].as_str().unwrap();
            if legacy != nativo {
                divergenti += 1;
            }
        }
        assert_eq!(
            divergenti, 7,
            "sette driver su dieci divergono fra legacy e nativo (dxf, filegdb, \
             geoparquet, gpkg, ipc, kml, xls): e' la ragione per cui lo split \
             esiste. Se questo numero cambia, o e' cambiato un driver o qualcuno \
             sta derivando il legacy dalla tripla"
        );
    }

    /// La matrice di handoff versionata è quella che il codice produce oggi.
    ///
    /// Snapshot, non ispezione: il file in `docs/contracts/` è ciò che chi
    /// mantiene `plenora-contracts` legge, e se divergesse dal codice senza che
    /// nessuno se ne accorgesse sarebbe peggio che non averlo — una matrice
    /// sbagliata si usa con la stessa fiducia di una giusta.
    ///
    /// Il test **non** rigenera il file da solo: fallisce e mostra la
    /// differenza, così l'aggiornamento resta una decisione di chi cambia il
    /// contratto invece di un effetto collaterale di `cargo test`.
    #[test]
    fn la_matrice_di_handoff_e_aggiornata() {
        let percorso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/contracts/handoff-plenora-error.json");
        let atteso =
            serde_json::to_string_pretty(&matrice_di_handoff()).expect("la matrice si serializza");
        let versionato = std::fs::read_to_string(&percorso)
            .unwrap_or_else(|error| panic!("{}: {error}", percorso.display()));
        assert_eq!(
            // Il confronto normalizza i soli CR: il file e' scritto con LF, ma
            // un checkout su Windows puo' restituirlo con CRLF, e la matrice
            // non e' diversa per questo.
            versionato.replace('\r', "").trim(),
            atteso.trim(),
            "la matrice versionata non corrisponde al codice: rigenerala"
        );
    }

    /// Ogni codice del vocabolario compare nella matrice.
    ///
    /// È la proprietà che rende l'elenco generato invece che copiato: una
    /// variante nuova di `IoErrorCode` che nessuno aggiunge a `TUTTI` viene
    /// presa qui, non da un lettore attento tre mesi dopo.
    #[test]
    fn il_vocabolario_dei_codici_e_completo() {
        use plenora_io_model::IoErrorCode;

        let matrice = matrice_di_handoff();
        let elencati = matrice["vocabolari"]["code"].as_array().unwrap().len();
        assert_eq!(
            elencati,
            IoErrorCode::TUTTI.len(),
            "la matrice elenca {elencati} codici, il tipo ne ha {}",
            IoErrorCode::TUTTI.len()
        );
        // E `TUTTI` copre davvero l'enum: un codice assente non serializzerebbe
        // mai, quindi si verifica che ogni voce sia una stringa distinta.
        let mut viste = std::collections::BTreeSet::new();
        for codice in matrice["vocabolari"]["code"].as_array().unwrap() {
            let nome = codice.as_str().expect("codice come stringa");
            assert!(viste.insert(nome), "codice duplicato: {nome}");
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
