//! plenora-io — CLI (Fase 2A). Comandi: `catalog` (registro driver), `inspect`
//! (formato + layer + schema + CRS), `layers` (elenco layer), `read` (scan +
//! conteggio righe), `convert` (pipeline operation-atomic: valida la sorgente
//! fino a EOF prima di esporre batch al writer, poi trasferisce i `RecordBatch`
//! e pubblica atomicamente). Nessuna riproiezione: il CRS è
//! letto/scritto, mai trasformato (`PRODUCT.md § CRS`).
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// Il gestore dei segnali sta in un file proprio perche' la sonda che prova
// l'uscita al secondo segnale lo compila a sua volta: vedi `segnali.rs`.
mod segnali;
use segnali::installa_gestore_dei_segnali;

// Le radici che l'artefatto porta con se': dove le librerie native trovano i
// propri dati quando il binario e' installato invece che compilato qui.
pub mod radici;

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
pub type CliResult = Result<Value, (i32, Value)>;

/// Le estensioni che la CLI riconosce. Vocabolario chiuso di letterali
/// nostri: puo' comparire in un messaggio pubblico.
const ESTENSIONI_AMMESSE: &str =
    "parquet, geojson, csv, gpkg, shp, kml, xlsx, xls, dxf, gdb, arrow";

/// I flag che la CLI riconosce. Stessa ragione.
/// Gli identificatori di formato che `--to` accetta, nell'ordine del catalogo.
///
/// Sta accanto a `ESTENSIONI_AMMESSE` e non lo sostituisce: sono due
/// vocabolari diversi, uno del filesystem e uno del catalogo, e confonderli e'
/// esattamente cio' che `--to` esiste per evitare.
const FORMATI_AMMESSI: &str = "geoparquet, geojson, csv, gpkg, shp, kml, xls, dxf, filegdb, ipc";

const OPZIONI_AMMESSE: &str = "--assume-crs, --deadline-ms, --durable, --in-opt, --layer,      --limit, --max-columns, --max-input-bytes, --max-input-entries,      --max-output-bytes, --max-rows, --max-vertices, --max-wkb-cell-bytes,      --format, --max-wkb-components, --max-wkb-depth, --memory-bytes, --opt,      --out-opt, --output, --from, --to, --version";

#[allow(clippy::cast_possible_truncation)]
const fn saturating_u64(value: usize) -> u64 {
    if usize::BITS > u64::BITS && value > u64::MAX as usize {
        u64::MAX
    } else {
        value as u64
    }
}

/// I tetti della diagnostica nella busta e il troncamento che li dichiara.
pub mod busta;
/// La superficie Rust pubblica: le sei operazioni del catalogo, per nome.
pub mod operazioni;

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
            // `details.row_diagnostics`, e non `error.row_diagnostics`.
            //
            // ROW-DIAGNOSTICS-1.0 lo dice per esteso: «When the enclosing
            // serialized error uses `error-v1.schema.json`, the complete
            // document is placed at `details.row_diagnostics`». Al primo
            // livello la chiave rendeva il documento d'errore **invalido**
            // contro quello schema, che dichiara `additionalProperties:
            // false` e non prevede `row_diagnostics`: una busta che nessun
            // consumatore poteva validare senza sapere che noi la scriviamo
            // diversa.
            //
            // Sotto `details` il documento entra anche nei tetti strutturali
            // di ERR-012, che al primo livello non lo toccavano affatto. Il
            // caso peggiore che il prodotto sa emettere -- 64 esempi, 64
            // categorie, stringhe ai loro tetti -- misura 53 531 byte, 601
            // nodi, profondita' 5 e 64 proprieta' per oggetto, contro tetti
            // di 262 144, 2 048, 8 e 128: ci sta per costruzione, e la
            // costruzione sono i tetti che il prodotto gia' applica.
            Ok(document) => error_document["details"] = json!({"row_diagnostics": document}),
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
    let busta_intera = json!({
        "status": "error",
        "protocol_version": busta::PROTOCOLLO,
        "contract": "plenora-error-v1",
        "error": error_document,
    });
    // ERR-011 misura la busta **intera**, e qui e' dove ogni busta d'errore
    // della libreria nasce. La riserva tiene conto dei campi d'identita' che il
    // binding aggiungera' dopo: senza, una busta al limite passerebbe di qui e
    // sforerebbe nel punto in cui nessuno guarda piu'.
    busta::entro_il_tetto_dell_errore(busta_intera, busta::RISERVA_IDENTITA).0
}

/// Un errore costruito qui, col codice d'uscita **derivato dalla categoria**.
///
/// # Perche' rende la coppia
///
/// Rendeva il solo documento, e ogni sito d'uso scriveva il proprio codice
/// d'uscita accanto: sei siti, sei numeri a mano, e tre di essi sbagliati
/// rispetto alla tabella del contratto -- `unsupported` usciva `4` dove il
/// contratto proietta `3`, e `not_found` usciva `1` dove proietta `5`.
///
/// Correggere i sei numeri avrebbe chiuso le istanze e lasciato la **classe**:
/// il settimo sito sarebbe nato con lo stesso difetto. Rendendo la coppia, il
/// codice non e' piu' una cosa che un sito puo' scrivere.
fn local_err_doc(
    code: &str,
    category: ErrorCategory,
    phase: ErrorPhase,
    message: &PublicMessage,
) -> (i32, Value) {
    let documento = err_doc(
        code,
        // `redatto` con `Generic`, non un costruttore di famiglia: il sito
        // usava `PlenoraIoError::new`, che imposta `code = Generic`. Un
        // costruttore di famiglia imporrebbe il proprio, cambiando il
        // quartetto — ed e' la regressione che il gate dei quartetti esiste
        // per fermare.
        &PlenoraIoError::redatto(
            plenora_io_model::IoErrorCode::Generic,
            category,
            phase,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        ),
    );
    (uscita_della_categoria(category), documento)
}

/// L'identificatore pubblico del componente.
///
/// La forma che `SURF-001` pretende e' `plenora-<domain>-tools`, tutta
/// minuscola. Il manifesto di protocollo scriveva `plenora-IO-tools`, che quella
/// forma non ammette, e nessuna busta lo esponeva affatto.
const COMPONENTE: &str = "plenora-io-tools";

/// Il `command` di una busta prodotta prima che un comando sia stato scelto.
///
/// Non e' un comando del binding, e non deve sembrarlo. Echeggiare cio' che il
/// chiamante ha scritto sarebbe peggio: metterebbe in un campo d'identita' un
/// valore che decide chi invoca.
pub const COMANDO_IGNOTO: &str = "unknown";

/// La busta comune di un esito riuscito, e l'unico posto che la costruisce.
///
/// I dati dell'operazione stanno **dentro** `result`. Prima uscivano al primo
/// livello accanto a `status` e `contract`, e un consumatore non poteva
/// distinguere un campo del protocollo da un campo del driver senza conoscerli
/// tutti e due.
#[must_use]
pub fn busta_di_successo(comando: &str, risultato: Value) -> Value {
    // La costruzione e' esplicita e non col macro: `json!` serializza il
    // risultato per riferimento, e il corpo di una conversione porta cinque
    // sezioni diagnostiche. Qui si **muove**.
    let mut busta = serde_json::Map::new();
    busta.insert("status".to_owned(), json!("ok"));
    busta.insert("protocol_version".to_owned(), json!(busta::PROTOCOLLO));
    busta.insert("component".to_owned(), json!(COMPONENTE));
    busta.insert(
        "component_version".to_owned(),
        json!(env!("CARGO_PKG_VERSION")),
    );
    busta.insert("contract".to_owned(), json!(busta::contratto(comando)));
    busta.insert("command".to_owned(), json!(comando));
    busta.insert("result".to_owned(), risultato);
    Value::Object(busta)
}

/// I campi d'identita' su una busta d'errore gia' costruita.
///
/// Si aggiungono qui e non in `err_doc` perche' il comando si conosce piu'
/// tardi: `err_doc` e' chiamata anche da dentro le operazioni, che il proprio
/// nome non lo sanno.
#[must_use]
pub fn con_identita(mut documento: Value, comando: &str) -> Value {
    if let Value::Object(campi) = &mut documento {
        campi.insert("component".to_owned(), json!(COMPONENTE));
        campi.insert(
            "component_version".to_owned(),
            json!(env!("CARGO_PKG_VERSION")),
        );
        campi.insert("command".to_owned(), json!(comando));
    }
    // Il secondo passaggio, ed e' quello che chiude il cerchio: qui la busta e'
    // **completa**, identita' compresa, ed e' l'oggetto che ERR-011 misura. Il
    // primo passaggio in `err_doc` lavora con una riserva, quindi normalmente
    // qui non resta niente da fare -- ma le buste che non passano da li',
    // come quella di un panico, passano da qui.
    busta::entro_il_tetto_dell_errore(documento, 0).0
}

/// Il codice d'uscita di una categoria, secondo CLI 2.0 §8.
///
/// # Il difetto che chiude
///
/// La proiezione era agganciata a `IoErrorCode` -- il codice **interno** -- e
/// non alla categoria, che e' l'asse che il contratto dichiara autoritativo.
/// Produceva `1` dove il contratto vuole `5`, `4` dove vuole `3`, e i valori `7`
/// e `8`, che nel contratto non esistono. L'unico ramo che coincideva era
/// `cancelled`, ed era l'unico gia' agganciato alla categoria.
///
/// Una categoria nuova senza mappatura esplicita proietta a `70` e mai a
/// successo: e' cio' che il contratto pretende, e il `match` esaustivo lo rende
/// una scelta invece che un caso.
#[must_use]
pub const fn uscita_della_categoria(categoria: ErrorCategory) -> i32 {
    match categoria {
        ErrorCategory::InvalidPlan | ErrorCategory::InvalidConfiguration => 2,
        ErrorCategory::Schema
        | ErrorCategory::DataMapping
        | ErrorCategory::Crs
        | ErrorCategory::Unsupported => 3,
        ErrorCategory::ResourceLimit => 4,
        ErrorCategory::Io
        | ErrorCategory::NotFound
        | ErrorCategory::Conflict
        | ErrorCategory::Protocol
        | ErrorCategory::Authentication
        | ErrorCategory::Authorization
        | ErrorCategory::Timeout
        | ErrorCategory::Transient => 5,
        ErrorCategory::Execution => 6,
        ErrorCategory::Internal => 70,
        ErrorCategory::Cancelled => 130,
    }
}

/// Un errore d'avvio come busta, con il codice d'uscita degli errori d'uso.
///
/// Non passa da `usage_err` perche' non e' un uso sbagliato: l'invocazione e'
/// corretta, ed e' l'ambiente a non permettere di onorarla. Il codice resta `2`
/// -- il comando non ha prodotto niente, e chi lo script-a lo tratta come gli
/// altri rifiuti che precedono il lavoro.
fn errore_di_avvio(errore: &PlenoraIoError) -> (i32, Value) {
    (
        uscita_della_categoria(errore.category),
        err_doc("CLI_STARTUP", errore),
    )
}

fn usage_err(message: &PublicMessage) -> (i32, Value) {
    local_err_doc(
        "CLI_USAGE",
        ErrorCategory::InvalidConfiguration,
        ErrorPhase::Validate,
        message,
    )
}

/// Mappa un `PlenoraIoError` a (exit, doc) con codici stabili.
// Usata come funzione in `map_err`/`unwrap_or_else`: la firma per valore è
// imposta dai punti di chiamata.
#[allow(clippy::needless_pass_by_value)]
fn map_err(e: plenora_io_model::PlenoraIoError) -> (i32, Value) {
    use plenora_io_model::IoErrorCode;
    // Il `code` resta il nostro, e resta agganciato a `IoErrorCode`: e' un
    // dettaglio strutturato accanto ai quattro assi, e il contratto lo ammette.
    // Cio' che non doveva essere agganciato al codice interno e' il **codice
    // d'uscita**, e ora non lo e' piu'.
    let code = match e.code {
        IoErrorCode::OutputExists => "OUTPUT_EXISTS",
        IoErrorCode::Unsupported | IoErrorCode::Capability => "UNSUPPORTED",
        IoErrorCode::Crs => "CRS_REQUIRED",
        IoErrorCode::Contract | IoErrorCode::Schema => "CONTRACT",
        IoErrorCode::LimitExceeded => "LIMIT_EXCEEDED",
        IoErrorCode::ReaderBusy => "READER_BUSY",
        IoErrorCode::ProjectionUnsupported => "PROJECTION_UNSUPPORTED",
        IoErrorCode::CrsUnresolved => "CRS_UNRESOLVED",
        _ => "FORMAT_ERROR",
    };
    let code = if e.category == ErrorCategory::Cancelled {
        "CANCELLED"
    } else {
        code
    };
    let document = err_doc(code, &e);
    // La diagnostica interna non conforme e' sostituita da un errore
    // `internal`, e il suo codice d'uscita segue quella categoria come ogni
    // altro: prima usciva `1`, che non e' la proiezione di niente.
    let categoria = if document["error"]["code"] == "INVALID_ROW_DIAGNOSTICS" {
        ErrorCategory::Internal
    } else {
        e.category
    };
    (uscita_della_categoria(categoria), document)
}

fn combined_fidelity(read: &FidelityAssessment, write: &FidelityAssessment) -> FidelityAssessment {
    let level = match (read.level, write.level) {
        (Fidelity::Approximating, _) | (_, Fidelity::Approximating) => Fidelity::Approximating,
        (Fidelity::Conditional, _) | (_, Fidelity::Conditional) => Fidelity::Conditional,
        (Fidelity::Lossless, Fidelity::Lossless) => Fidelity::Lossless,
    };
    // `merge` e non `add_reason(code, detail)`: ricostruire ogni ragione dai due
    // campi perderebbe la posizione **e** la frase congelata del v1, quindi la
    // sezione di conversione cambierebbe byte in tutt'e due i protocolli. E le
    // due collezioni si fondono ciascuna con la propria regola: il v1 sulla
    // chiave vecchia, il v2 su quella canonica.
    let mut combined = FidelityAssessment::con_livello(level);
    combined.merge(read);
    combined.merge(write);
    combined
}

/// Il rapporto di perdita, nella forma del protocollo corrente.
///
/// Un `BudgetInsufficiente` non degrada a una busta senza diagnostica: e' un
/// errore, e la sola cosa peggiore di una diagnostica troncata e' una troncata
/// che tace.
fn loss_doc(fidelity: &FidelityAssessment, loss: &LossReport) -> Result<Value, (i32, Value)> {
    busta::documento_di_perdita(fidelity, loss).map_err(|_| budget_err())
}

/// La valutazione di fedelta', nella forma del protocollo corrente.
fn fidelity_doc(fidelity: &FidelityAssessment) -> Result<Value, (i32, Value)> {
    busta::documento_di_fedelta(fidelity).map_err(|_| budget_err())
}

/// Il budget della sezione non basta nemmeno alla dichiarazione.
fn budget_err() -> (i32, Value) {
    local_err_doc(
        "DIAGNOSTIC_BUDGET_EXHAUSTED",
        ErrorCategory::Internal,
        ErrorPhase::Validate,
        &PublicMessage::Curated(
            "il budget della diagnostica non basta nemmeno alla dichiarazione di troncamento",
        ),
    )
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
            return Err(local_err_doc(
                    "XLS_BINARY_UNSUPPORTED",
                    ErrorCategory::Unsupported,
                    ErrorPhase::Validate,
                    &PublicMessage::Curated(
                        "capability drop esplicita: il contenitore binario BIFF .xls                          non e' supportato; usare .xlsx",
                    ),
                ),
            )
        }
        "dxf" => Box::new(driver_dxf::DxfDriver),
        "gdb" => Box::new(driver_filegdb::FileGdbDriver),
        "arrow" => Box::new(driver_ipc::IpcDriver),
        // Legato a `_`: l'estensione non serve piu' a nessuno qui, ed e' la
        // prova — in forma di binding — che non entra nel messaggio.
        _ => {
            return Err(local_err_doc(
                    "UNSUPPORTED",
                    ErrorCategory::Unsupported,
                    ErrorPhase::Validate,
                    &PublicMessage::CuratedPair(
                        "estensione non riconosciuta; ammesse:",
                        ESTENSIONI_AMMESSE,
                    ),
                ),
            )
        }
    };
    Ok(d)
}

// --- parsing argomenti ------------------------------------------------------

#[derive(Default)]
pub struct Cli {
    pub(crate) positionals: Vec<String>,
    /// Dove `read` consegna il dataset Arrow.
    ///
    /// E' un flag e non un posizionale perche' la consegna e' **facoltativa**:
    /// `read SORGENTE` legge e riporta senza materializzare, ed e' il modo in
    /// cui si valida una sorgente grande. Un secondo posizionale avrebbe reso
    /// la consegna obbligatoria o la sua assenza indistinguibile da un refuso.
    pub(crate) output: Option<PathBuf>,
    pub(crate) assume_crs: Option<String>,
    /// Il formato del sink di `write`, **nominato**.
    ///
    /// Non dedotto dall'estensione della destinazione: il profilo io-tools dice
    /// che il comportamento specifico di un formato non va scelto analizzando
    /// l'estensione quando l'operazione richiede un formato esplicito, e il
    /// catalogo descrive `io.write` come «publish an Arrow dataset to an
    /// **explicit** sink format». Un'estensione e' una convenzione del
    /// filesystem, non un identificatore di formato: `.json` e' `GeoJSON` qui e
    /// mille altre cose altrove, e una destinazione senza estensione non
    /// avrebbe risposta.
    ///
    /// I valori ammessi sono gli `id` che `io.catalog` rende, e una prova lo
    /// verifica nei due versi: nessun formato del catalogo senza driver, e
    /// nessun driver che il catalogo non dichiari.
    pub(crate) to: Option<String>,
    /// Il formato della **sorgente** di `convert`, nominato.
    ///
    /// Esiste per la stessa ragione di `to`, e la ragione e' nel catalogo:
    /// `io.convert` vi e' descritto come «convert an external dataset between
    /// **explicit** formats». Due formati, due nomi.
    ///
    /// Non vale per `inspect`, `layers` e `read`, e non e' una dimenticanza:
    /// quelle operazioni **riconoscono** la sorgente invece di riceverla
    /// dichiarata -- `io.inspect` rende «its **declared** format» -- e il
    /// suffisso li' e' l'unico segnale che c'e'. Riconoscere e dichiarare sono
    /// due cose, e il catalogo le distingue operazione per operazione.
    pub(crate) from_: Option<String>,
    pub(crate) layer: Option<u32>,
    pub(crate) limit: Option<usize>,
    pub(crate) durable: bool,
    pub(crate) opts: BTreeMap<String, String>,
    pub(crate) in_opts: BTreeMap<String, String>,
    pub(crate) out_opts: BTreeMap<String, String>,
    /// I flag di quota, gia' nel tipo del modello unificato.
    ///
    /// Fino a S4.d atterravano in un `Limits` legacy e venivano tradotti piu'
    /// tardi. Il tipo intermedio non serviva a nulla se non a tenere in vita
    /// il modello vecchio nel punto piu' visibile del componente.
    pub(crate) limits: PipelineLimits,
    /// Il token che il gestore dei segnali arma.
    ///
    /// `parse` ne mette uno **nuovo e non armato**: un token e' un canale, e
    /// fabbricarlo qui non lega niente a niente. Chi lo lega e' `run`, che
    /// sostituisce quello del processo dopo il parsing. Un test che chiama
    /// `parse` ottiene percio' un token isolato, e non puo' annullare per
    /// sbaglio l'operazione di un altro test.
    pub(crate) cancellazione: CancellationToken,
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
        // La deadline della pipeline, finora fissata a 30 000 ms nel modello e
        // non raggiungibile da riga di comando. Sta con le altre quote perche'
        // e' una quota: governa il tempo come `--max-rows` governa le righe, e
        // condivide la disciplina fail-closed del gruppo — lo zero lo rifiuta
        // `PipelineLimits::validate`, un valore che sposta la scadenza oltre
        // cio' che `Instant` rappresenta lo rifiuta `build`. Nessuno dei due
        // degrada al default.
        "--deadline-ms" => limiti.with_duration_ms(parse_u64(it.next(), "--deadline-ms")?),
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

/// Legge gli argomenti in una `Cli`.
///
/// # Errors
///
/// `Err` quando un argomento manca, e' ripetuto o non e' del tipo atteso: la
/// busta ha categoria `invalid_configuration` e codice `CLI_USAGE`, e l'`i32`
/// e' la proiezione che il binding applichera'.
pub fn parse(args: &[String]) -> Result<Cli, (i32, Value)> {
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
            // Il selettore del modo macchina e' accettato da ogni comando, ed
            // e' l'unico valore ammesso: `--format` con altro non e' una
            // modalita' che non abbiamo, e' un refuso che fallisce chiuso.
            FLAG_FORMATO => {
                let v = it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--format richiede un valore"))
                })?;
                if v != FORMATO_JSON {
                    return Err(usage_err(&PublicMessage::Curated(
                        "--format ammette soltanto json",
                    )));
                }
            }
            "--output" => {
                let v = it.next().ok_or_else(|| {
                    usage_err(&PublicMessage::Curated("--output richiede un percorso"))
                })?;
                cli.output = Some(PathBuf::from(v));
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
            "--from" => {
                cli.from_ = Some(
                    it.next()
                        .ok_or_else(|| {
                            usage_err(&PublicMessage::Curated(
                                "--from richiede un identificatore di formato",
                            ))
                        })?
                        .clone(),
                );
            }
            "--to" => {
                cli.to = Some(
                    it.next()
                        .ok_or_else(|| {
                            usage_err(&PublicMessage::Curated(
                                "--to richiede un identificatore di formato",
                            ))
                        })?
                        .clone(),
                );
            }
            "--durable" => cli.durable = true,
            // Il nome dice che cosa si sta scegliendo. `--protocol 1` sarebbe
            // stato piu' corto e avrebbe fatto sembrare le due versioni due
            // opzioni pari: non lo sono.
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
    let bundle = PipelineBudget::builder()
        .limits(cli.limits)
        .cancellation(cli.cancellazione.clone())
        .build()?;
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
    let bundle = PipelineBudget::builder()
        .limits(cli.limits)
        .cancellation(cli.cancellazione.clone())
        .build()?;
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
    //
    // `--in-opt` entra qui, e prima non entrava. Il difetto e' emerso
    // scrivendo `plenora-io-read-input-v1`: il parser accettava `--in-opt`, il
    // testo d'uso la elencava fra le ammesse, e `read`, `inspect` e `layers` la
    // **scartavano** -- solo `convert` la univa a `--opt`. Una chiave sbagliata
    // passava in silenzio, che e' il caso peggiore: indistinguibile da una
    // applicata. Con `opts_uniti` il driver la vede, e se non la conosce
    // risponde `unsupported` come gia' fa per `--opt`.
    //
    // La precedenza e' quella dichiarata dal README e fissata da
    // `opts_uniti_preserva_precedenza_direzionale`: la direzionale sovrascrive
    // la comune, per chiave.
    let mut opzioni = read_pipeline(cli)?.with_format_options(opts_uniti(&cli.opts, &cli.in_opts));
    opzioni.assume_crs.clone_from(&cli.assume_crs);
    Ok(opzioni)
}

// --- comandi ----------------------------------------------------------------

/// Che cosa questo binario espone, e che cosa non espone.
///
/// # La regola che governa questo documento
///
/// Descrive **l'artefatto che sta rispondendo**, non il perimetro che il
/// catalogo comune definisce. Un'operazione che il catalogo dichiara richiesta e
/// che questo binario non serve va dichiarata `unavailable` con una ragione
/// leggibile da una macchina, non omessa e tantomeno annunciata: un documento
/// che promette cio' che non c'e' e' peggio di nessun documento, perche' un
/// orchestratore lo crede.
///
/// # Perche' non elenca i formati
///
/// Il profilo lo dice per esteso: il risultato versionato di `io.catalog` e' la
/// **sola** fonte normativa per gli identificatori di formato, le opzioni
/// ammesse, la disponibilita' in lettura e scrittura, il comportamento dei
/// layer, la geometria, il CRS e i vincoli di fedelta'; e gli `attributes` non
/// devono duplicare quella matrice. Qui si dice che `io.catalog` esiste e come
/// si invoca, e il resto lo dice lui.
///
/// Ne segue una cosa che sorprende: la build `base` e quella `gdal-backend`
/// producono lo **stesso** documento. La feature non cambia quali operazioni
/// esistono -- cambia quali formati `io.catalog` dichiara disponibili, e quella
/// e' una domanda sua.
///
/// # Perche' i contratti dicono `-v2`
///
/// Il catalogo comune fissa `plenora-io-catalog-v1` e le altre undici forme; il
/// binario emette `-v2`. Qui si dichiara cio' che **esce davvero**: un documento
/// capability che nominasse i contratti del catalogo mentre il binario ne emette
/// altri sarebbe falso proprio nel punto in cui un consumatore si fida. La
/// divergenza e' una deviazione da registrare nel manifesto di adozione, non da
/// nascondere qui.
/// Un'operazione che questo binario serve davvero.
///
/// # Perche' due superfici e non una
///
/// Fino alla superficie Rust, `["cli"]` era vero: le operazioni vivevano dentro
/// un binario e un binario non si importa. Ora `plenora_io_tools::operazioni`
/// espone le stesse sei per nome, con lo stesso risultato e lo stesso errore --
/// e' la proprieta' che `tests/equivalenza_superfici.rs` misura -- quindi un
/// documento che ne dichiarasse una sola direbbe meno del vero a chi sceglie
/// come invocarci.
fn operazione_esposta(
    id: &str,
    ingresso: &str,
    uscita: &str,
    effetto: &str,
    controlli: bool,
) -> Value {
    json!({
        "id": id,
        "version": 1,
        "status": "available",
        "surfaces": ["cli", "rust"],
        "input": { "contract": ingresso, "content_types": ["application/json"] },
        "output": { "contract": uscita, "content_types": ["application/json"] },
        "side_effect": effetto,
        "controls": {
            "cancellation": controlli,
            "deadline": controlli,
            "idempotency_key": false,
        },
    })
}

/// Un'operazione che il catalogo comune pretende e che questo binario non serve.
///
/// # Perche' e' sparita, e perche' la regola resta
///
/// C'era `operazione_assente`, e la usava `io.write`. Con `io.write`
/// implementata tutte e sei le operazioni del catalogo sono `available`, e una
/// funzione che nessuno chiama e' codice che nessuno prova: il lint la boccia,
/// e ha ragione.
///
/// La regola che quella funzione serviva non e' sparita con lei, ed e' del
/// profilo: un artefatto rilasciato **puo'** omettere un'operazione che non fa
/// parte di quell'artefatto, ma **non deve** annunciarla disponibile. Il giorno
/// in cui una build parziale -- senza un driver, senza una feature -- non
/// servisse una delle sei, il documento dovra' dichiararla `unavailable` con la
/// ragione, non tacerla e non dirla disponibile. La forma sta in questo
/// commento e in `git log`, che e' dove va tenuto cio' che non ha chiamanti.
#[must_use]
pub fn capabilities_document() -> Value {
    json!({
        "schema_version": 2,
        "component": COMPONENTE,
        "component_version": env!("CARGO_PKG_VERSION"),
        "interfaces": [
            {
                "kind": "cli",
                "contract": "plenora-cli-v2",
                "version": busta::PROTOCOLLO,
                "artifact": "plenora-io",
            },
            // La superficie Rust, che dalla 4.0.0 esiste davvero. CAP-007
            // pretende che ogni superficie dichiarata da un'operazione sia
            // anche fra le interfacce dell'artefatto: dichiarare `rust` sulle
            // operazioni e tacerlo qui e' esattamente cio' che il verificatore
            // ha respinto, e aveva ragione -- un consumatore avrebbe letto una
            // superficie senza il contratto che la descrive.
            //
            // Il contratto e' **nostro**: SURFACE-BINDINGS-1.0 §2 non
            // prescrive nomi Rust e chiede invece che ogni componente pubblichi
            // una mappatura versionata da operazione a export pubblico. Quella
            // mappatura e' `contracts/superficie-rust.json`, e questo e' il suo
            // identificatore.
            {
                "kind": "rust",
                "contract": "plenora-io-rust-v1",
                "version": 1,
                "artifact": "plenora-io-tools",
            }
        ],
        "operations": [
            operazione_esposta(
                "io.catalog",
                "plenora-io-catalog-query-v1",
                "plenora-io-catalog-v1",
                "none",
                false,
            ),
            operazione_esposta(
                "io.inspect",
                "plenora-io-inspect-input-v1",
                "plenora-io-inspect-v1",
                "none",
                true,
            ),
            operazione_esposta(
                "io.layers",
                "plenora-io-layers-input-v1",
                "plenora-io-layers-v1",
                "none",
                true,
            ),
            // `io.read` consegna, e la sua uscita non e' JSON: e' l'unica
            // operazione disponibile che produce Arrow, ed e' la ragione per
            // cui non passa da `operazione_esposta`.
            //
            // `side_effect` e' `local` e non `none`: con `--output` scrive un
            // file, e un effetto che il documento tacesse sarebbe un effetto
            // che chi orchestra non si aspetta. La forma senza consegna non
            // scrive niente, ma il descrittore dichiara il **massimo** rischio,
            // non quello del caso migliore.
            json!({
                "id": "io.read",
                "version": 1,
                "status": "available",
                "surfaces": ["cli", "rust"],
                "input": {
                    "contract": "plenora-io-read-input-v1",
                    "content_types": ["application/json"],
                },
                "output": {
                    "contract": "plenora-io-read-result-v1",
                    "content_types": ["application/vnd.apache.arrow.file"],
                    // Il catalogo comune lo dichiara, e noi lo rispettiamo: le
                    // undici sonde di `metadati_arrow.rs` leggono dal file
                    // consegnato cio' che ARROW-001..012 pretende. Ometterlo
                    // qui diceva **meno** del vero -- un consumatore che
                    // cercasse il contratto d'interscambio non lo trovava, e
                    // avrebbe concluso che il payload non ne segue nessuno.
                    "interchange_contracts": ["plenora-arrow-interchange-v1"],
                },
                "side_effect": "local",
                "controls": {
                    "cancellation": true,
                    "deadline": true,
                    "idempotency_key": false,
                },
                "attributes": {
                    "materialization": "bounded",
                    "delivery": "operation_atomic",
                    "nota": "diagnostica opaca (CAP-013): la selezione si fa sui content type, non su queste chiavi. `materialization: bounded` e' la dichiarazione che ARROW-011 ammette esplicitamente («unless the operation descriptor explicitly declares bounded materialization»); `delivery: operation_atomic` dice quando il primo batch diventa visibile, ed e' vero per tutti e dieci i driver: se una violazione emerge in un punto qualsiasi della sorgente l'operazione e' rifiutata come blocco unico. Le due cose rispondono a domande diverse. Il catalogo comune ammette per io.read anche application/vnd.apache.arrow.stream, che e' una **serializzazione** e non una consegna incrementale: un file in formato stream si puo' produrre per intero prima di consegnarlo, quindi l'atomicita' non lo esclude e non richiede un protocollo nuovo. Questa superficie non lo annuncia perche' non produce una seconda serializzazione, ed e' una scelta di prodotto: il restringimento di una voce required e' registrato come deviazione nel manifesto di adozione, come PUBLIC-CATALOGS-1.0 §5 prescrive."
                },
            }),
            // `io.write` accetta un dataset Arrow e ne pubblica uno esterno,
            // quindi i content type d'ingresso sono due: il documento dei
            // parametri e il payload. Su questa superficie il payload arriva
            // come **file** e non come stream, per la ragione speculare a
            // quella di `io.read`: CLI-2.0 §4 riserva stdout alla busta, e
            // stdin a un solo documento non basta a portare i due.
            json!({
                "id": "io.write",
                "version": 1,
                "status": "available",
                "surfaces": ["cli", "rust"],
                "input": {
                    "contract": "plenora-io-write-input-v1",
                    "content_types": [
                        "application/json",
                        "application/vnd.apache.arrow.file",
                    ],
                    "interchange_contracts": ["plenora-arrow-interchange-v1"],
                },
                "output": {
                    "contract": "plenora-io-write-result-v1",
                    "content_types": ["application/json"],
                },
                "side_effect": "local",
                "controls": {
                    "cancellation": true,
                    "deadline": true,
                    "idempotency_key": false,
                },
            }),
            operazione_esposta(
                "io.convert",
                "plenora-io-convert-input-v1",
                "plenora-io-convert-v1",
                "local",
                true,
            ),
        ],
    })
}

/// `capabilities` accetta soltanto il selettore esplicito del modo macchina.
fn parse_capabilities(argomenti: &[String]) -> Result<(), (i32, Value)> {
    match argomenti {
        [] => Ok(()),
        [uno, due] if uno == FLAG_FORMATO && due == FORMATO_JSON => Ok(()),
        _ => Err(usage_err(&PublicMessage::Curated(
            "capabilities non prende argomenti, salvo --format json",
        ))),
    }
}

fn catalog_document_con(filegdb_available: bool) -> Value {
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

        "determinism": "byte_for_byte",
        "drivers": drivers,
    })
}

/// `catalog` accetta soltanto il selettore esplicito del modo macchina.
///
/// Fino alla 3.0.0 accettava anche `--legacy-protocol-v1-unsafe`, che sceglieva
/// il protocollo congelato. Il profilo pubblico vieta a un artefatto di servire
/// due versioni del protocollo JSON, e il flag e' stato tolto: ora e' un flag
/// sconosciuto, e un flag sconosciuto fallisce chiuso come gli altri.
fn parse_catalog(argomenti: &[String]) -> Result<(), (i32, Value)> {
    match argomenti {
        [] => Ok(()),
        [uno, due] if uno == FLAG_FORMATO && due == FORMATO_JSON => Ok(()),
        _ => Err(usage_err(&PublicMessage::Curated(
            "catalog non prende argomenti, salvo --format json",
        ))),
    }
}

#[must_use]
pub fn cmd_catalog() -> Value {
    catalog_document_con(driver_filegdb::runtime_available())
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

/// `io.inspect`. La porta pubblica e' `operazioni::inspect`.
///
/// # Errors
///
/// La busta d'errore del contratto comune quando la sorgente non si apre, il
/// formato non si riconosce o le opzioni non sono ammesse.
pub fn cmd_inspect(cli: &Cli) -> CliResult {
    let (driver, path) = open_source(cli)?;
    let ropts = read_options(cli).map_err(map_err)?;
    let ds = driver.open(Source::Path(path), ropts).map_err(map_err)?;
    let fidelity = ds.fidelity_assessment();
    let layers: Vec<Value> = ds.layers().iter().map(layer_json).collect();
    Ok(json!({

        "format": serde_json::to_value(driver.descriptor()).unwrap_or(Value::Null),
        "fidelity": fidelity_doc(&fidelity)?,
        "layers": layers,
    }))
}

/// `io.layers`. La porta pubblica e' `operazioni::layers`.
///
/// # Errors
///
/// Come `cmd_inspect`: la sorgente e' lo stesso ingresso.
pub fn cmd_layers(cli: &Cli) -> CliResult {
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

        "format": driver.descriptor().id(),
        "fidelity": fidelity_doc(&fidelity)?,
        "layers": layers,
    }))
}

/// La richiesta di lettura, con **il** token del processo e non uno nuovo.
///
/// Il campo portava `CancellationToken::default()`, cioe' un token fresco per
/// ogni richiesta: sintatticamente valido e semanticamente inerte, perche'
/// nessuno poteva annullarlo. Il reader interrogava un canale che non aveva un
/// altro capo.
fn read_request(cli: &Cli, layer_id: u32, scope: ReadScope) -> ReadRequest {
    ReadRequest {
        layer: plenora_io_model::contract::LayerId(layer_id),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope,
        batch_target: BatchTarget::default(),
        cancellation: cli.cancellazione.clone(),
    }
}

/// Il nome pubblico di un esito di pubblicazione.
///
/// Una funzione e non due `match`: erano due, uno in `convert` e uno in `read`,
/// e due `match` sullo stesso enum divergono al primo variante nuovo -- il
/// compilatore li obbliga entrambi a coprirlo, non a dargli lo stesso nome.
const fn esito_di_pubblicazione(esito: PublishOutcome) -> &'static str {
    match esito {
        PublishOutcome::Published => "published",
        PublishOutcome::PublishedButDurabilityUnconfirmed => "published_durability_unconfirmed",
    }
}

/// La forma di `io.read` che legge e **non** consegna.
///
/// Non e' una lettura a meta': e' una lettura completa di cui si conserva il
/// giudizio invece dei dati, ed e' il modo in cui si risponde a «questa
/// sorgente si legge?» senza pagare il disco.
fn legge_senza_consegnare(
    cli: &Cli,
    ds: &dyn plenora_io_core::driver::OpenDatasetHandle,
    driver: &dyn FormatDriver,
    contract: &LayerContract,
    fidelity_iniziale: &FidelityAssessment,
    layer_id: u32,
    scopo: ReadScope,
) -> CliResult {
    let mut reader = ds
        .open_layer_reader(&read_request(cli, layer_id, scopo))
        .map_err(map_err)?;
    let (mut rows, mut batches) = (0usize, 0usize);
    while let Some(batch) = reader.next_batch().map_err(map_err)? {
        rows += batch.num_rows();
        batches += 1;
        if cli.limit.is_some_and(|l| rows >= l) {
            break;
        }
    }
    let perdita = reader.loss_report();
    Ok(json!({
        "format": driver.descriptor().id(),
        "fidelity": fidelity_doc(&fidelity_iniziale.clone().with_loss_report(&perdita))?,
        "loss": loss_doc(fidelity_iniziale, &perdita)?,
        "layer": layer_json(contract),
        "rows_read": rows,
        "batches": batches,
        "truncated": cli.limit.is_some_and(|l| rows >= l),
        "delivered": Value::Null,
    }))
}

/// Le combinazioni d'argomenti che `io.read` non accetta.
///
/// Estratta da `cmd_read` perche' quella funzione ha un tetto di righe, ma
/// anche perche' qui sta una cosa sola: la validazione dell'ingresso, che e'
/// esattamente cio' che `contracts/schemas/plenora-io-read-input-v1.schema.json`
/// dichiara. Le due devono rifiutare lo stesso insieme, e
/// `tests/schemi_io_read.rs` lo verifica invocando il binario sugli esempi
/// invalidi dello schema.
fn ingresso_ammissibile(cli: &Cli) -> Result<(), (i32, Value)> {
    // Opzioni indirizzate a un sink che non esistera'.
    //
    // Senza `--output` non si crea nessun writer, quindi `--out-opt` e
    // `--durable` non hanno destinatario: venivano accettate e cadevano nel
    // vuoto. Con `--output` il driver le vede e rifiuta le chiavi che non
    // conosce -- il rifiuto c'era gia', mancava soltanto nel caso in cui non
    // c'e' nessuno a cui chiedere.
    //
    // E' lo stesso principio che governa `--in-opt`: un'opzione accettata e
    // senza effetto e' indistinguibile da una applicata, e chi la scrive
    // crede di aver cambiato qualcosa.
    if cli.output.is_none() && (!cli.out_opts.is_empty() || cli.durable) {
        return Err(local_err_doc(
            "SINK_OPTIONS_WITHOUT_SINK",
            ErrorCategory::InvalidConfiguration,
            ErrorPhase::Validate,
            &PublicMessage::Curated(
                "--out-opt e --durable descrivono la destinazione, e senza \
                 --output non c'e' destinazione da descrivere. Con --output \
                 valgono, e il driver rifiuta le chiavi che non conosce.",
            ),
        ));
    }

    if cli.limit.is_some() && cli.output.is_some() {
        return Err(local_err_doc(
            "LIMIT_WITH_DELIVERY",
            ErrorCategory::InvalidPlan,
            ErrorPhase::Validate,
            &PublicMessage::Curated(
                "--limit e --output insieme non sono ammessi: io.read non \
                 consegna dataset parziali, e le due semantiche possibili del \
                 totale sono incompatibili. La scelta e' scritta in \
                 plenora-io-read-input-v1. Senza --output il limite vale, e la \
                 lettura riporta quante righe ha visto.",
            ),
        ));
    }

    Ok(())
}

/// Il driver di un formato **nominato**, non dedotto.
///
/// # Perche' non basta `driver_for_path`
///
/// Quella funzione guarda l'estensione della destinazione, ed e' la cosa che il
/// profilo io-tools vieta per le operazioni che richiedono un formato
/// esplicito. Non e' una finezza formale: un'estensione e' una convenzione del
/// filesystem che non appartiene a nessun vocabolario -- `.json` e' `GeoJSON` qui
/// e mille altre cose altrove -- mentre l'identificatore di formato e' una voce
/// del catalogo versionato, cioe' una cosa di cui il componente risponde.
///
/// I nomi sono gli `id` che `io.catalog` rende.
/// `ogni_formato_del_catalogo_ha_un_driver` li confronta nei due versi: un
/// formato annunciato e non scrivibile sarebbe una capacita' falsa, e un driver
/// che il catalogo non nomina sarebbe una capacita' nascosta.
fn driver_per_formato(id: &str) -> Result<Box<dyn FormatDriver>, (i32, Value)> {
    let driver: Box<dyn FormatDriver> = match id {
        "geoparquet" => Box::new(driver_geoparquet::GeoParquetDriver),
        "geojson" => Box::new(driver_geojson::GeoJsonDriver),
        "csv" => Box::new(driver_csv::CsvDriver),
        "gpkg" => Box::new(driver_gpkg::GpkgDriver),
        "shp" => Box::new(driver_shp::ShpDriver),
        "kml" => Box::new(driver_kml::KmlDriver),
        "xls" => Box::new(driver_xls::XlsDriver),
        "dxf" => Box::new(driver_dxf::DxfDriver),
        "filegdb" => Box::new(driver_filegdb::FileGdbDriver),
        "ipc" => Box::new(driver_ipc::IpcDriver),
        // Legato a `_`: il nome arriva da argv e non entra nel messaggio, che
        // porta invece l'elenco chiuso di cio' che si puo' chiedere.
        _ => {
            return Err(local_err_doc(
                "UNKNOWN_FORMAT",
                ErrorCategory::Unsupported,
                ErrorPhase::Validate,
                &PublicMessage::CuratedPair(
                    "formato non riconosciuto; gli identificatori sono quelli di \
                     io.catalog:",
                    FORMATI_AMMESSI,
                ),
            ))
        }
    };
    Ok(driver)
}

/// I layer dell'ingresso che vanno pubblicati.
///
/// `None` significa **tutti**, e non e' incoerente con `io.read`, dove
/// l'assenza di `--layer` significa «il solo»: li' si legge un layer e
/// sceglierne uno implicitamente sarebbe arbitrario, qui si pubblica un
/// dataset e pubblicarlo intero e' il caso normale.
fn strati_da_pubblicare(
    disponibili: &[LayerContract],
    scelto: Option<u32>,
) -> Result<Vec<LayerContract>, (i32, Value)> {
    let Some(id) = scelto else {
        return Ok(disponibili.to_vec());
    };
    disponibili
        .iter()
        .find(|l| l.id.0 == id)
        .cloned()
        .map(|l| vec![l])
        .ok_or_else(|| {
            local_err_doc(
                "LAYER_NOT_FOUND",
                ErrorCategory::InvalidPlan,
                ErrorPhase::Validate,
                &PublicMessage::Curated("il layer richiesto non esiste nell'ingresso"),
            )
        })
}

/// Il piano di scrittura: un layer del sink per ciascun layer selezionato,
/// nello stesso ordine, perche' e' l'ordine su cui `trasferisci_layer` indicizza.
fn piano_di_scrittura(selezionati: &[LayerContract]) -> WritePlan {
    WritePlan {
        layers: selezionati
            .iter()
            .map(|l| WriteLayer {
                name: l.name.clone(),
                contract: DataContract {
                    schema: l.contract.schema.clone(),
                    geometry: l.contract.geometry.clone(),
                },
            })
            .collect(),
    }
}

/// `io.write`: pubblica un dataset Arrow in un formato **nominato**.
///
/// # Che cosa la distingue da `convert`
///
/// L'ingresso non e' una sorgente qualsiasi: e' un dataset Arrow, cioe' la
/// forma in cui `io.read` consegna. Le due operazioni sono l'una l'inversa
/// dell'altra, e questo e' il motivo per cui condividono il contratto
/// d'interscambio: cio' che esce da `read --output` deve poter entrare qui.
///
/// `convert` prende due sorgenti esterne e le collega; `write` prende il
/// dataset che il chiamante **ha gia' in mano** e lo pubblica. Un orchestratore
/// che leggesse, trasformasse e riscrivesse non potrebbe usare `convert`: fra i
/// due passi ci sono i suoi dati, non un file di cui ci occupiamo noi.
///
/// # Le fedelta' sono due, e la distinzione non e' cosmetica
///
/// `input_fidelity` e' quel che la lettura del file Arrow ha potuto conservare
/// -- normalmente tutto, perche' Arrow e' la rappresentazione e non una
/// traduzione -- mentre `write_fidelity` e' quel che il formato di
/// destinazione puo' esprimere. Attribuire al sink una perdita dell'ingresso,
/// o viceversa, direbbe a chi legge di cambiare la cosa sbagliata. `fidelity`
/// e' il giudizio combinato, che e' quello dell'operazione.
/// # Errors
///
/// La busta d'errore del contratto comune. Fra i casi propri: `UNKNOWN_FORMAT`
/// per un `--to` fuori dal catalogo, l'errore d'uso senza `--to`, e i rifiuti
/// di capacita' del formato di destinazione.
pub fn cmd_write(cli: &Cli) -> CliResult {
    if cli.positionals.len() < 2 {
        return Err(usage_err(&PublicMessage::Curated(
            "write richiede <ingresso.arrow> <destinazione> --to <formato>",
        )));
    }
    let Some(formato) = cli.to.as_deref() else {
        // Un default qui sarebbe la scelta implicita che il profilo vieta:
        // meglio un rifiuto che nomina la mancanza.
        return Err(usage_err(&PublicMessage::Curated(
            "write richiede --to <formato>: il formato del sink e' esplicito, \
             e non si deduce dall'estensione della destinazione",
        )));
    };

    let in_path = PathBuf::from(&cli.positionals[0]);
    let out_path = PathBuf::from(&cli.positionals[1]);
    let sorgente = driver_ipc::IpcDriver;
    let sink = driver_per_formato(formato)?;

    let (mut ropts, mut wopts) = convert_pipeline(cli).map_err(map_err)?;
    ropts.assume_crs.clone_from(&cli.assume_crs);
    ropts.format_options = opts_uniti(&cli.opts, &cli.in_opts);
    let ds = sorgente
        .open(Source::Path(in_path), ropts)
        .map_err(map_err)?;
    let fedelta_ingresso = ds.fidelity_assessment();

    let selezionati = strati_da_pubblicare(ds.layers(), cli.layer)?;
    let piano = piano_di_scrittura(&selezionati);
    wopts.durable = cli.durable;
    wopts.format_options = opts_uniti(&cli.opts, &cli.out_opts);
    let mut writer = sink
        .create(Sink::Path(out_path), &piano, &wopts)
        .map_err(map_err)?;

    let mut righe_totali = 0usize;
    let mut perdita_in_ingresso = LossReport::default();
    let mut rapporti = Vec::new();
    for (indice, strato) in selezionati.iter().enumerate() {
        let mut reader = ds
            .open_layer_reader(&read_request(cli, strato.id.0, ReadScope::Complete))
            .map_err(map_err)?;
        let sink_layer =
            plenora_io_model::contract::LayerId(u32::try_from(indice).map_err(|_| {
                map_err(PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "numero di layer non rappresentabile",
                )))
            })?);
        let (righe, batch) =
            trasferisci_layer(reader.as_mut(), writer.as_mut(), sink_layer).map_err(map_err)?;
        perdita_in_ingresso.merge(&reader.loss_report());
        righe_totali = righe_totali.checked_add(righe).ok_or_else(|| {
            map_err(PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "overflow nel conteggio delle righe scritte",
            )))
        })?;
        rapporti.push(json!({"name": strato.name, "rows": righe, "batches": batch}));
    }
    let pubblicato = writer.finish().map_err(map_err)?;

    let fedelta_ingresso = fedelta_ingresso.with_loss_report(&perdita_in_ingresso);
    let combinata = combined_fidelity(&fedelta_ingresso, &pubblicato.fidelity);

    let ingresso = fidelity_doc(&fedelta_ingresso)?;
    let scrittura = fidelity_doc(&pubblicato.fidelity)?;
    let complessiva = fidelity_doc(&combinata)?;
    let perdita_ingresso = loss_doc(&fedelta_ingresso, &perdita_in_ingresso)?;
    let perdita_scrittura = loss_doc(&pubblicato.fidelity, &pubblicato.loss)?;
    busta::diagnostica_entro_il_totale(&[
        ("input_fidelity", &ingresso),
        ("write_fidelity", &scrittura),
        ("fidelity", &complessiva),
        ("input_loss", &perdita_ingresso),
        ("write_loss", &perdita_scrittura),
    ])
    .map_err(|_| budget_err())?;

    Ok(json!({
        "format": sink.descriptor().id(),
        "input": {
            "content_type": "application/vnd.apache.arrow.file",
            "interchange_contract": "plenora-arrow-interchange-v1",
        },
        "layers": rapporti,
        "rows_written": righe_totali,
        "bytes_written": pubblicato.bytes,
        "publish_outcome": esito_di_pubblicazione(pubblicato.outcome),
        "fidelity": complessiva,
        "input_fidelity": ingresso,
        "write_fidelity": scrittura,
        "input_loss": perdita_ingresso,
        "write_loss": perdita_scrittura,
    }))
}

/// `io.read`: legge un layer e, quando glielo si chiede, lo **consegna**.
///
/// # Che cosa e' cambiato, e perche' era un difetto
///
/// Fino alla 3.0.0 questo comando apriva il lettore, drenava i batch contandoli
/// e li **scartava**. Rendeva righe, batch e fedelta': informazione vera e
/// utile, ma non i dati. Il catalogo comune definisce `io.read` come la lettura
/// di un layer «as an Arrow dataset plus structured fidelity evidence», e
/// dichiara per la sua uscita i content type Arrow: un'operazione che non
/// consegna non e' quell'operazione, ed e' la ragione per cui il documento
/// capability la dichiarava non disponibile.
///
/// # Le due forme, e perche' restano due
///
/// Senza `--output` il comportamento e' quello di prima: si legge, si conta, si
/// riporta la fedelta'. Serve a chi vuole **sapere** senza materializzare, ed e'
/// il modo in cui si validano sorgenti grandi. Con `--output` si consegna.
///
/// Le due forme si distinguono nel risultato: `delivered` porta il content type
/// prodotto, i byte e il percorso quando c'e' una consegna, ed e' assente
/// quando non c'e'. Un consumatore non deve dedurre dall'assenza di un campo se
/// i dati esistano: glielo dice `delivered`.
/// # Errors
///
/// La busta d'errore del contratto comune. Fra i casi propri di questa
/// operazione: `LIMIT_WITH_DELIVERY` quando il limite accompagna la consegna,
/// `SINK_OPTIONS_WITHOUT_SINK` quando le opzioni del sink arrivano senza
/// destinazione, e i rifiuti di capacita' del sink Arrow.
pub fn cmd_read(cli: &Cli) -> CliResult {
    let (driver, path) = open_source(cli)?;
    let ropts = read_options(cli).map_err(map_err)?;
    let ds = driver.open(Source::Path(path), ropts).map_err(map_err)?;
    let fidelity_iniziale = ds.fidelity_assessment();
    let layer_id = cli.layer.unwrap_or(0);
    let contract = ds
        .layers()
        .iter()
        .find(|l| l.id.0 == layer_id)
        .ok_or_else(|| {
            local_err_doc(
                "NO_LAYER",
                ErrorCategory::NotFound,
                ErrorPhase::Prepare,
                &PublicMessage::CuratedWith(
                    "layer inesistente all'indice",
                    NumeroStrutturale::Indice(u64::from(layer_id)),
                ),
            )
        })?
        .clone();

    let scopo = cli.limit.map_or(ReadScope::Complete, |limit| {
        ReadScope::AcceptedRows(limit as u64)
    });

    // `--limit` con `--output`: rifiutato per **scelta scritta**, non per
    // obbligo del contratto e non per comodita' del writer.
    //
    // # Che cosa il contratto lasciava a noi
    //
    // Nessun requisito di `plenora-contracts@453c8d1` vieta una consegna
    // parziale. `SURF-014` vieta di **riportare** un esito parziale come
    // successo pieno -- e la busta lo riporta gia' con `truncated` --, mentre
    // `PUBLIC-SURFACES-1.0 §9` punto 5 delega «success, partial and failure
    // semantics» alla specifica dell'operazione. Quella specifica e' nostra:
    // `contracts/schemas/plenora-io-read-input-v1.schema.json`, dove la
    // decisione e' scritta per esteso.
    //
    // # La decisione, e perche' non e' il writer a dettarla
    //
    // `declare_input_total` vuole la cardinalita' esatta, ma un vincolo di
    // implementazione e' una ragione per progettare la semantica, non per
    // negarla. La ragione sta in **questo** contratto d'uscita, che ha un
    // totale solo. Con un totale solo le due letture si escludono:
    //
    // * «il totale e' quello della sorgente e ne consegno N» conserva il
    //   denominatore delle diagnostiche di riga, e consegna un dataset che non
    //   gli corrisponde: chi lo rilegge conta N righe mentre il documento ne
    //   dichiara di piu';
    // * «il totale e' N» rende coerente il file e cancella l'informazione su
    //   quanto e' rimasto indietro -- che e' precisamente cio' per cui il
    //   limite era stato chiesto.
    //
    // Non e' una proprieta' del problema: e' una proprieta' della forma che
    // abbiamo scelto. Un contratto che rappresentasse **separatamente** le
    // righe della sorgente e quelle consegnate direbbe entrambe le cose senza
    // che nessuna cancelli l'altra, e allora una consegna parziale sarebbe
    // esprimibile. Rifiutare adesso non chiude quella porta: la tiene, perche'
    // un rifiuto si toglie mentre una semantica gia' consegnata no.
    //
    // «Le prime N righe come dataset» resta intanto un'operazione legittima e
    // **diversa**: e' una proiezione, non una lettura limitata. Fino ad allora
    // `limit` governa quante righe si **leggono**, non quante se ne
    // consegnano.
    ingresso_ammissibile(cli)?;

    let Some(uscita) = cli.output.clone() else {
        return legge_senza_consegnare(
            cli,
            ds.as_ref(),
            driver.as_ref(),
            &contract,
            &fidelity_iniziale,
            layer_id,
            scopo,
        );
    };

    // La consegna. Il sink e' **sempre** Arrow IPC: `io.read` consegna un
    // dataset Arrow, e lasciare che l'estensione scegliesse il formato avrebbe
    // reso `read` un secondo `convert` con un nome diverso.
    let sink = driver_ipc::IpcDriver;
    let piano = WritePlan {
        layers: vec![WriteLayer {
            name: contract.name.clone(),
            contract: DataContract {
                schema: contract.contract.schema.clone(),
                geometry: contract.contract.geometry.clone(),
            },
        }],
    };
    // Le quote vengono dal budget unificato, come per `convert`: costruirne un
    // altro qui darebbe a `read` limiti diversi da quelli che il chiamante ha
    // chiesto.
    let (_, mut wopts) = convert_pipeline(cli).map_err(map_err)?;
    wopts.durable = cli.durable;
    wopts.format_options = opts_uniti(&cli.opts, &cli.out_opts);
    let mut writer = sink
        .create(Sink::Path(uscita.clone()), &piano, &wopts)
        .map_err(map_err)?;

    let mut reader = ds
        .open_layer_reader(&read_request(cli, layer_id, scopo))
        .map_err(map_err)?;
    let (rows, batches) = trasferisci_layer(
        reader.as_mut(),
        writer.as_mut(),
        plenora_io_model::contract::LayerId(0),
    )
    .map_err(map_err)?;
    let perdita = reader.loss_report();
    let pubblicato = writer.finish().map_err(map_err)?;

    // La fedelta' della **lettura**, non della conversione.
    //
    // `io.read` legge: cio' che il sink IPC potrebbe perdere non e' una perdita
    // di questa operazione, ed e' anche il caso che non si presenta -- Arrow e'
    // la rappresentazione, non una traduzione. Riportare una fedelta' combinata
    // attribuirebbe alla lettura qualcosa che la lettura non ha fatto.
    let fedelta = fidelity_iniziale.clone().with_loss_report(&perdita);
    // I byte del file consegnato. `map_or` e non `map().unwrap_or()`: la
    // seconda forma costruisce un `Result` intermedio per buttarlo via.
    let byte = std::fs::metadata(&uscita).map_or(0, |m| m.len());

    Ok(json!({
        "format": driver.descriptor().id(),
        "fidelity": fidelity_doc(&fedelta)?,
        "loss": loss_doc(&fidelity_iniziale, &perdita)?,
        "layer": layer_json(&contract),
        "rows_read": rows,
        "batches": batches,
        "truncated": cli.limit.is_some_and(|l| rows >= l),
        "delivered": {
            "content_type": "application/vnd.apache.arrow.file",
            "interchange_contract": "plenora-arrow-interchange-v1",
            "bytes_written": byte,
            "publish_outcome": esito_di_pubblicazione(pubblicato.outcome),
        },
    }))
}

/// Trasferisce un layer dal reader al writer **in streaming**, trattenendo al
/// massimo un batch per volta.
///
/// # Perché non basta leggere tutto e poi scrivere
///
/// Fino a questa revisione la CLI accumulava in un `Vec` tutti i batch del
/// layer prima del primo write, e lo faceva per una ragione vera:
/// `declare_input_total` esige la cardinalità **prima** del primo write del
/// layer (`plenora-io-core/src/driver.rs`), e per conoscerla sembrava servisse
/// aver già letto tutto. Il costo era `O(dataset)` di memoria, e cadeva
/// esattamente dove la libreria aveva speso più fatica per non pagarlo: sotto
/// la CLI, l'adapter operation-atomic tiene già i batch verificati in uno
/// `StagedSpool` che migra su disco oltre soglia, con picco indipendente dalla
/// dimensione dell'input. La CLI riportava in RAM ciò che lo spool aveva appena
/// tolto — e con esso il `--memory-bytes` che l'operatore aveva scelto.
///
/// # La sequenza
///
/// 1. il **primo** `next_batch` conclude l'esame bounded dell'intero scope: è
///    lì che l'atomicità operativa viene pagata, ed è lì che il totale diventa
///    un fatto;
/// 2. `accepted_total` restituisce quel totale. Se non lo restituisce si
///    **fallisce chiuso**: nessun ripiego che riaccumuli il layer, perché un
///    ripiego silenzioso rimetterebbe il difetto dove era, visibile solo a chi
///    misura la memoria;
/// 3. `declare_input_total` riceve il totale prima di qualunque write;
/// 4. da lì si alternano write e letture, un batch per volta, e il batch
///    corrente viene rilasciato **prima** di chiedere il successivo.
///
/// L'alternanza del punto 4 è la proprietà che rende bounded il consumo, e non
/// si deduce dalla memoria misurata: una lettura che precede la scrittura del
/// batch corrente consuma il doppio senza che nessun contatore lo dica. È per
/// questo che la sonda la osserva direttamente.
///
/// # Errors
///
/// Se il reader non sa dichiarare il totale accettato, se un conteggio
/// trabocca, o se lettura e scrittura falliscono.
fn trasferisci_layer(
    reader: &mut dyn plenora_io_core::driver::LayerReader,
    writer: &mut dyn plenora_io_core::driver::FormatWriter,
    sink_layer: plenora_io_model::contract::LayerId,
) -> Result<(usize, usize), PlenoraIoError> {
    let mut corrente = reader.next_batch()?;
    let input_total = reader.accepted_total().ok_or_else(|| {
        PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
            "il reader non dichiara la cardinalita' accettata: conversione rifiutata",
        ))
    })?;
    writer.declare_input_total(sink_layer, input_total)?;
    let (mut rows, mut batches) = (0usize, 0usize);
    while let Some(batch) = corrente.take() {
        rows = rows.checked_add(batch.num_rows()).ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "overflow nel conteggio righe CLI",
            ))
        })?;
        batches = batches.checked_add(1).ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "overflow nel conteggio batch CLI",
            ))
        })?;
        writer.write_to_layer(sink_layer, &batch)?;
        // Rilascio esplicito prima della lettura successiva: senza di esso il
        // picco sarebbe di due batch, e la sonda dell'alternanza resterebbe
        // verde su un codice che occupa il doppio.
        drop(batch);
        corrente = reader.next_batch()?;
    }
    Ok((rows, batches))
}

// La pipeline di conversione è operation-atomic: apertura, validazione fino a
// EOF, trasferimento batch e pubblicazione devono restare in un'unica sequenza
// leggibile, con i fallimenti nell'ordine esatto in cui la CLI li espone.
#[allow(clippy::too_many_lines)]
/// # Errors
///
/// La busta d'errore del contratto comune. Fra i casi propri: l'errore d'uso
/// senza i due formati espliciti, `UNKNOWN_FORMAT` per un identificatore fuori
/// dal catalogo, e i rifiuti di capacita' dei due formati.
pub fn cmd_convert(cli: &Cli) -> CliResult {
    if cli.positionals.len() < 2 {
        return Err(usage_err(&PublicMessage::Curated(
            "convert richiede <ingresso> <uscita> --from <formato> --to <formato>",
        )));
    }
    // I due formati sono **nominati**, e dalla 4.0.0 non si deducono piu' dalle
    // estensioni.
    //
    // # Che cosa cambia per chi invocava `convert` prima
    //
    // Ogni invocazione esistente li omette, quindi ogni invocazione esistente
    // va aggiornata: e' una rottura, ed e' dichiarata in `docs/INSTALL.md`
    // insieme alla tabella di corrispondenza fra estensioni e identificatori.
    // Non c'e' un periodo di grazia in cui l'assenza continui a dedurre: un
    // default che sopravvive alla deprecazione e' la deprecazione che non
    // avviene.
    //
    // # Perche' si rompe invece di dedurre
    //
    // Il catalogo comune descrive `io.convert` come operazione «between
    // **explicit** formats», e il profilo io-tools vieta di scegliere il
    // comportamento specifico di un formato analizzando l'estensione quando
    // l'operazione richiede un formato esplicito. Dedurre era comodo e diceva
    // una cosa falsa: che `.json` significhi GeoJSON, che un percorso senza
    // estensione non abbia formato, e che il nome di un file sia un contratto.
    let (Some(nome_ingresso), Some(nome_uscita)) = (cli.from_.as_deref(), cli.to.as_deref()) else {
        return Err(usage_err(&PublicMessage::Curated(
            "convert richiede --from <formato> e --to <formato>: i due formati \
             sono espliciti e non si deducono dalle estensioni. Gli \
             identificatori sono quelli di io.catalog",
        )));
    };
    let in_path = PathBuf::from(&cli.positionals[0]);
    let out_path = PathBuf::from(&cli.positionals[1]);
    let src = driver_per_formato(nome_ingresso)?;
    let dst = driver_per_formato(nome_uscita)?;
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
            local_err_doc(
                "NO_LAYER",
                ErrorCategory::NotFound,
                ErrorPhase::Prepare,
                &PublicMessage::CuratedWith(
                    "layer inesistente all'indice",
                    NumeroStrutturale::Indice(u64::from(id)),
                ),
            )
        })?],
        None => all,
    };
    // Multi-layer verso destinazione single-layer: vietato (fail-closed).
    if selected.len() > 1 && !dst.descriptor().multi_layer() {
        return Err(local_err_doc(
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
        );
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
            .open_layer_reader(&read_request(cli, l.id.0, ReadScope::Complete))
            .map_err(map_err)?;
        let sink_layer =
            plenora_io_model::contract::LayerId(u32::try_from(sink_idx).map_err(|_| {
                map_err(PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "numero di layer sorgente non rappresentabile",
                )))
            })?);
        let (rows, batches) =
            trasferisci_layer(reader.as_mut(), writer.as_mut(), sink_layer).map_err(map_err)?;
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

    let outcome = esito_di_pubblicazione(published.outcome);
    // Le cinque sezioni si costruiscono prima, perche' il tetto complessivo si
    // verifica sull'insieme: i tetti per sezione non delimitano l'aggregato.
    let lettura = fidelity_doc(&read_fidelity)?;
    let scrittura = fidelity_doc(&published.fidelity)?;
    let conversione = fidelity_doc(&conversion_fidelity)?;
    let perdita_in_lettura = loss_doc(&read_fidelity, &read_loss)?;
    let perdita_in_scrittura = loss_doc(&published.fidelity, &published.loss)?;
    // Il tetto complessivo vale sempre: era condizionato al v2 perche' il v1
    // congelato non aveva budget da rispettare, e con un protocollo solo la
    // condizione non ha piu' un secondo caso.
    busta::diagnostica_entro_il_totale(&[
        ("read_fidelity", &lettura),
        ("write_fidelity", &scrittura),
        ("conversion_fidelity", &conversione),
        ("read_loss", &perdita_in_lettura),
        ("write_loss", &perdita_in_scrittura),
    ])
    .map_err(|_| budget_err())?;
    Ok(json!({

        "from": src.descriptor().id(),
        "to": dst.descriptor().id(),
        "layers": layer_reports,
        "total_rows": total_rows,
        "bytes_written": published.bytes,
        "publish_outcome": outcome,
        "read_fidelity": lettura,
        "write_fidelity": scrittura,
        "conversion_fidelity": conversione,
        "read_loss": perdita_in_lettura,
        "write_loss": perdita_in_scrittura,
    }))
}

/// `parse`, piu' il legame con il token del processo.
///
/// Sta separata da `parse` perche' `parse` e' la funzione che le sonde
/// esercitano, e non deve dipendere da uno stato di processo per essere
/// verificabile.
/// Come `parse`, ma lega il token di cancellazione agli argomenti.
///
/// # Errors
///
/// Gli stessi di `parse`: un argomento che manca, si ripete o non e' del tipo
/// atteso.
pub fn parse_legato(
    args: &[String],
    cancellazione: &CancellationToken,
) -> Result<Cli, (i32, Value)> {
    let mut cli = parse(args)?;
    cli.cancellazione = cancellazione.clone();
    Ok(cli)
}

/// Il selettore esplicito del modo macchina, e il suo unico valore.
///
/// CLI 2.0 vuole che il modo JSON si scelga, e che non lo scelga un
/// orchestratore per implicito. Accettarlo su ogni comando costa una riga e
/// toglie l'ambiguita': senza, un consumatore non puo' dichiarare che cosa si
/// aspetta.
const FLAG_FORMATO: &str = "--format";
const FORMATO_JSON: &str = "json";

/// L'esito di un comando: il nome canonico e il documento, o l'errore.
///
/// Il nome viaggia con l'esito perche' la busta lo richiede e chi costruisce il
/// corpo non lo sa: `cmd_inspect` produce il proprio `result`, non la propria
/// identita'.
type EsitoDelComando = (&'static str, CliResult);

/// Il testo di `--help`.
///
/// Descrive **soltanto** i comandi compilati in questo binario. `catalog` dice
/// che cosa c'e' davvero -- i driver e le capability della build -- e resta la
/// via per saperlo con precisione; qui si elencano i comandi, non le capacita'.
pub const AIUTO: &str = "\
plenora-io — lettura, scrittura e conversione di dataset esterni.

USO
    plenora-io <comando> [argomenti] [--format json]

COMANDI
    capabilities       le operazioni che questo binario espone, e quali no
    catalog            i formati e le capability di questo binario
    inspect SORGENTE   formato, layer, schemi e fedelta' dichiarati
    layers SORGENTE    i layer indirizzabili della sorgente
    read SORGENTE      legge un layer e riporta fedelta' e conteggi
    write IN OUT --to F  pubblica un dataset Arrow nel formato F
    convert IN OUT --from F --to G   converte fra due formati espliciti

SCOPERTA
    --help             questo testo
    --version          la versione del componente, nella busta comune

Ogni comando emette un documento JSON su stdout e niente su stderr. Il codice
d'uscita e' la proiezione della categoria d'errore: 0 riuscito, 2 configurazione
non valida, 3 schema, mappatura, CRS o non supportato, 4 limite di risorse, 5
I/O e famiglia, 6 esecuzione, 70 interno, 130 annullato.
";

pub fn run() -> EsitoDelComando {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Il gestore si installa **prima** del dispatch, non dentro il comando: un
    // Ctrl+C battuto mentre la sorgente si apre deve trovare il token gia'
    // armato.
    //
    // Se non si installa, il comando non parte: la ragione sta su
    // `RIFIUTO_SEGNALI`, e in breve e' che un avviso testuale finirebbe davanti
    // alla busta e romperebbe il contratto di `stderr`.
    let cancellazione = match installa_gestore_dei_segnali() {
        Ok(token) => token,
        Err(errore) => return (COMANDO_IGNOTO, Err(errore_di_avvio(&errore))),
    };

    if let Some(nome @ ("inspect" | "layers" | "read" | "write" | "convert")) =
        args.first().map(String::as_str)
    {
        let cli = match parse_legato(&args[1..], &cancellazione) {
            Ok(cli) => cli,
            Err(errore) => return (nome_canonico(nome), Err(errore)),
        };
        let esito = match nome {
            "inspect" => cmd_inspect(&cli),
            "layers" => cmd_layers(&cli),
            "read" => cmd_read(&cli),
            "write" => cmd_write(&cli),
            _ => cmd_convert(&cli),
        };
        return (nome_canonico(nome), esito);
    }

    if args.first().map(String::as_str) == Some("catalog") {
        return ("catalog", parse_catalog(&args[1..]).map(|()| cmd_catalog()));
    }

    if args.first().map(String::as_str) == Some("capabilities") {
        return (
            "capabilities",
            parse_capabilities(&args[1..]).map(|()| capabilities_document()),
        );
    }

    match args.first().map(String::as_str) {
        Some("--help" | "-h") if args.len() == 1 => ("help", Ok(Value::Null)),
        Some("--help" | "-h") => (
            "help",
            Err(usage_err(&PublicMessage::Curated(
                "--help non prende argomenti",
            ))),
        ),
        // `--version` accetta il selettore del modo macchina come gli altri, e
        // risponde nella busta comune. Fino alla 3.0.0 rendeva
        // `{"status":"ok","version":"…"}`, che busta non era: nessuna versione
        // di protocollo, nessuna identita', e la versione fuori da `result`.
        Some("--version" | "-V")
            if args.len() == 1
                || (args.len() == 3 && args[1] == FLAG_FORMATO && args[2] == FORMATO_JSON) =>
        {
            (
                "version",
                Ok(json!({
                    "component_version": env!("CARGO_PKG_VERSION"),
                    "cli_protocol_version": busta::PROTOCOLLO,
                })),
            )
        }
        Some("--version" | "-V") => (
            "version",
            Err(usage_err(&PublicMessage::Curated(
                "--version prende al piu' --format json",
            ))),
        ),
        _ => (
            COMANDO_IGNOTO,
            Err(usage_err(&PublicMessage::Curated(
                "uso: plenora-io <catalog|inspect|layers|read|convert> [--format json] \
                 | --help | --version",
            ))),
        ),
    }
}

/// Il nome canonico di un comando, come lo dichiara il binding.
///
/// Oggi coincide con cio' che il chiamante scrive, e la funzione esiste perche'
/// il campo `command` della busta e' un'**identita'** e non un'eco: il giorno in
/// cui un alias deprecato sara' ammesso, dovra' risolvere al nome canonico qui e
/// non finire nella busta come e' stato scritto.
const fn nome_canonico(invocato: &str) -> &'static str {
    match invocato.as_bytes() {
        b"inspect" => "inspect",
        b"layers" => "layers",
        b"read" => "read",
        b"write" => "write",
        b"convert" => "convert",
        _ => COMANDO_IGNOTO,
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
pub fn installa_hook_silenzioso() {
    std::panic::set_hook(Box::new(|_| {}));
}

pub fn envelope_panico(payload: &(dyn std::any::Any + Send)) -> Value {
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
        "contract": "plenora-error-v1",
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
        "generato_da": "plenora-io-tools, test la_matrice_di_handoff_e_aggiornata",
        "sorgente": {
            "contract": "plenora-error-v1",
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
            "assi_stabili": ["category", "phase", "code", "retry"],
            // Dove sta la busta, non solo com'e' fatta.
            //
            // Un consumatore la legge da `stderr` con un parser sull'intero
            // flusso: se qualcosa la precedesse, il parser si fermerebbe sulla
            // prima riga. E' successo -- un avviso emesso all'avvio finiva
            // davanti alla busta di un comando che poi falliva -- e il vincolo
            // e' qui perche' chi integra lo legga invece di scoprirlo.
            "stderr_su_errore": "esattamente un documento JSON, e nient'altro",
            "stderr_su_successo": "vuoto, salvo l'avviso del protocollo legacy quando e' stato scelto esplicitamente e la consegna e' riuscita",
            "stdout_su_successo": "il documento del comando; l'avviso del legacy non ci entra, perche' il v1 e' congelato byte per byte"
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod conformance_tests;
