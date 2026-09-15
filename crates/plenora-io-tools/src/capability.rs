//! Il documento capability e quello del catalogo: che cosa questo binario espone.
//!
//! Estratto da `lib.rs` per M1. Il gruppo aveva **un solo** riferimento in
//! entrata dai comandi -- `catalog_document_con` -- e nessuno in uscita verso
//! di loro: e' il confine piu' netto del file, e separarlo rende visibile una
//! dipendenza che prima era una chiamata fra vicini.
//!
//! Lo spostamento e' meccanico. Nessuna firma cambia, nessun ordine di
//! operazioni cambia, e `capabilities_document` resta raggiungibile dove era --
//! `lib.rs` lo riesporta.

use serde_json::{json, Value};

use plenora_io_core::DriverRegistry;
use plenora_io_model::PublicMessage;

use crate::busta;
use crate::{usage_err, COMPONENTE, FLAG_FORMATO, FORMATO_JSON};

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

/// Il descrittore di `io.read`, che non passa da `operazione_esposta`.
///
/// E' l'unica operazione disponibile la cui uscita non e' JSON, e l'unica
/// che dichiari attributi: sta in una funzione sua perche' e' cresciuta
/// abbastanza da non stare comoda dentro il documento che la contiene.
fn descrittore_di_read() -> Value {
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
            // Le due serializzazioni che Arrow IPC registra, e che
            // questa superficie ora produce entrambe. L'ordine segue
            // il catalogo comune; il default resta il contenitore, e
            // il flusso si chiede con `--out-opt serialization=stream`.
            "content_types": [
                "application/vnd.apache.arrow.stream",
                "application/vnd.apache.arrow.file",
            ],
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
            "nota": "diagnostica opaca (CAP-013): la selezione si fa sui content type, non su queste chiavi. `materialization: bounded` e' la dichiarazione che ARROW-011 ammette esplicitamente («unless the operation descriptor explicitly declares bounded materialization»), ed e' la forma in cui questa superficie soddisfa quel requisito ora che annuncia anche lo stream. `delivery: operation_atomic` dice quando il primo batch diventa visibile, ed e' vero per tutti e dieci i driver: se una violazione emerge in un punto qualsiasi della sorgente l'operazione e' rifiutata come blocco unico. Le due cose rispondono a domande diverse, e produrre un flusso non cambia la seconda: i byte si scrivono per intero nello staging e la pubblicazione resta l'ultima operazione. Quale delle due serializzazioni esca lo sceglie l'opzione di formato `serialization` del driver IPC, che `io.catalog` pubblica."
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
            descrittore_di_read(),
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
                        // Il payload arriva in una delle due serializzazioni, e
                        // quale sia lo dicono i suoi byte -- non il nome del
                        // file. `io.write` le legge entrambe.
                        "application/vnd.apache.arrow.stream",
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
pub fn parse_capabilities(argomenti: &[String]) -> Result<(), (i32, Value)> {
    match argomenti {
        [] => Ok(()),
        [uno, due] if uno == FLAG_FORMATO && due == FORMATO_JSON => Ok(()),
        _ => Err(usage_err(&PublicMessage::Curated(
            "capabilities non prende argomenti, salvo --format json",
        ))),
    }
}

pub fn catalog_document_con(filegdb_available: bool) -> Value {
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
pub fn parse_catalog(argomenti: &[String]) -> Result<(), (i32, Value)> {
    match argomenti {
        [] => Ok(()),
        [uno, due] if uno == FLAG_FORMATO && due == FORMATO_JSON => Ok(()),
        _ => Err(usage_err(&PublicMessage::Curated(
            "catalog non prende argomenti, salvo --format json",
        ))),
    }
}
