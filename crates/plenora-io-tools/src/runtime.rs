//! La superficie runtime: Runtime Binding 1.0 per le sei operazioni.
//!
//! # Che cosa e', e che cosa non e'
//!
//! Il confine **neutrale rispetto al trasporto** che RUNTIME-BINDING-1.0
//! descrive: un'invocazione serializzata entra -- metadati d'instradamento,
//! content type, payload JSON -- e un risultato serializzato esce, con il
//! contratto d'uscita e la correlazione dell'invocazione. Code, worker,
//! handler e il trasporto stesso sono dell'applicazione che ospita il
//! binding (runtime-tools o un'altra), e questo modulo non dipende da nessuno
//! di loro (RT-015).
//!
//! Le operazioni sono **le stesse** delle altre due superfici: ogni ramo
//! costruisce una [`Richiesta`] e chiama la funzione di
//! [`crate::operazioni`] che la CLI e la superficie Rust chiamano. Che il
//! risultato e l'errore coincidano non e' una proprieta' da verificare caso
//! per caso: e' la stessa chiamata. Cio' che appartiene al binding, e sta
//! qui, e' solo questo:
//!
//! * l'ammissione: identita' dei messaggi, instradamento, content type e
//!   controlli, tutti verificati **prima** di aprire qualunque cosa (RT-004,
//!   RT-005, RT-006, RT-011, RT-012);
//! * la lettura del payload contro lo schema `*-input-v1` dell'operazione, a
//!   campi chiusi e senza `null` letti come assenza;
//! * i riferimenti agli artefatti: `source` e `destination` non sono percorsi
//!   ma riferimenti opachi, che risolve l'applicazione attraverso un
//!   [`RisolutoreArtefatti`] (RT-013, RT-015);
//! * la scadenza assoluta di `plenora.execution.deadline`, che entra nello
//!   **stesso** tetto di `deadline_ms` e di `--deadline-ms` (RT-007);
//! * la consegna di `io.read`: il dataset Arrow esce come flusso nel
//!   risultato, in memoria, e non come file scritto da qualche parte.
//!
//! # Perche' `io.read` qui non ha effetti
//!
//! Sulla CLI `io.read` consegna scrivendo un file, perche' CLI-2.0 §4 riserva
//! stdout a un solo documento JSON e i byte Arrow non hanno altra strada: e'
//! la ragione per cui il descrittore della CLI dichiara `side_effect: local`.
//! Qui la strada c'e' -- il risultato **e'** il flusso -- e il binding la
//! prende: il payload non ammette `destination`, e il descrittore runtime
//! dichiara `none`, come il catalogo comune. Il file intermedio che la
//! consegna atomica usa vive in una directory temporanea privata del
//! processo, che nessun chiamante nomina e che sparisce prima della risposta.
//!
//! # La memoria
//!
//! Il flusso consegnato e' un `Vec<u8>`: il suo tetto e' `max_output_bytes`
//! della pipeline, lo stesso che governa il file della CLI. Un consumatore
//! che non voglia tenerlo in memoria non ha bisogno di questa superficie: la
//! CLI e la superficie Rust consegnano su file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Map, Value};

use plenora_io_model::budget::PipelineLimits;
use plenora_io_model::{ErrorCategory, ErrorPhase, PublicMessage};

/// Il canale di cancellazione che l'applicazione passa a [`BindingRuntime::invoca`].
///
/// Riesportato qui perche' un'applicazione che ospita il binding non debba
/// dipendere da `plenora-io-model`, che e' un crate interno e instabile.
pub use plenora_io_model::CancellationToken;

use crate::operazioni::{self, Richiesta};
use crate::{local_err_doc, COMPONENTE};

/// L'identita' della capacita' runtime (RT-001).
pub const NOME_CAPACITA: &str = "plenora.io-tools";
/// La versione del binding runtime (RT-002), indipendente dalla release.
pub const VERSIONE_BINDING: u32 = 1;
/// Il contratto che l'interfaccia runtime dichiara nel documento capability.
pub const CONTRATTO_BINDING: &str = "plenora-runtime-binding-v1";
/// Il content type dei payload JSON, in ingresso e in uscita.
pub const CONTENT_TYPE_JSON: &str = "application/json";
/// Il content type del flusso Arrow che `io.read` consegna.
pub const CONTENT_TYPE_FLUSSO_ARROW: &str = "application/vnd.apache.arrow.stream";
/// Il content type di un errore terminale.
pub const CONTENT_TYPE_ERRORE: &str = "application/vnd.plenora.error+json";
/// Il contratto di un errore terminale.
pub const CONTRATTO_ERRORE: &str = "plenora-error-v1";

/// Di quanto la durata della pipeline supera la scadenza assoluta.
const MARGINE_DELLA_DURATA_MS: u64 = 1_000;

/// La lunghezza massima di un riferimento ad artefatto, in caratteri.
///
/// La stessa di `plenora-data-execution-input-v3`, l'unico schema comune che
/// a questo pin fissa la forma di un riferimento: due componenti che
/// accettassero riferimenti di forma diversa renderebbero impossibile a chi
/// orchestra passare l'uscita dell'uno all'ingresso dell'altro.
const MAX_CARATTERI_RIFERIMENTO: usize = 2048;

/// Che cosa fa il binding con gli artefatti di un'operazione.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Artefatti {
    /// Nessun artefatto: `io.catalog`.
    Nessuno,
    /// Solo una sorgente: `io.inspect`, `io.layers`, `io.read`.
    Sorgente,
    /// Una sorgente e una destinazione: `io.write`, `io.convert`.
    SorgenteEDestinazione,
}

/// Il descrittore statico di un'operazione runtime, usato per l'ammissione.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DescrittoreRuntime {
    /// L'identificatore stabile dell'operazione (RT-003).
    pub operazione: &'static str,
    /// La versione del contratto dell'operazione.
    pub versione: u32,
    /// Il contratto d'ingresso.
    pub contratto_ingresso: &'static str,
    /// Il contratto d'uscita.
    pub contratto_uscita: &'static str,
    /// Il content type del risultato riuscito.
    pub content_type_uscita: &'static str,
    /// L'effetto dichiarato **su questa superficie**: `none` o `local`.
    pub effetto: &'static str,
    /// Gli artefatti che l'operazione apre.
    pub artefatti: Artefatti,
    /// Se l'operazione osserva la cancellazione e la scadenza.
    ///
    /// Le due vanno insieme in tutte e sei, e `io.catalog` non ne osserva
    /// nessuna: descrive l'artefatto che risponde e non apre nulla.
    pub controlli: bool,
}

const fn descrittore(
    operazione: &'static str,
    contratto_ingresso: &'static str,
    contratto_uscita: &'static str,
    content_type_uscita: &'static str,
    effetto: &'static str,
    artefatti: Artefatti,
    controlli: bool,
) -> DescrittoreRuntime {
    DescrittoreRuntime {
        operazione,
        versione: 1,
        contratto_ingresso,
        contratto_uscita,
        content_type_uscita,
        effetto,
        artefatti,
        controlli,
    }
}

/// Le sei operazioni, nell'ordine del catalogo comune.
pub const OPERAZIONI: [DescrittoreRuntime; 6] = [
    descrittore(
        "io.catalog",
        "plenora-io-catalog-query-v1",
        "plenora-io-catalog-v1",
        CONTENT_TYPE_JSON,
        "none",
        Artefatti::Nessuno,
        false,
    ),
    descrittore(
        "io.inspect",
        "plenora-io-inspect-input-v1",
        "plenora-io-inspect-v1",
        CONTENT_TYPE_JSON,
        "none",
        Artefatti::Sorgente,
        true,
    ),
    descrittore(
        "io.layers",
        "plenora-io-layers-input-v1",
        "plenora-io-layers-v1",
        CONTENT_TYPE_JSON,
        "none",
        Artefatti::Sorgente,
        true,
    ),
    descrittore(
        "io.read",
        "plenora-io-read-input-v1",
        "plenora-io-read-result-v1",
        CONTENT_TYPE_FLUSSO_ARROW,
        "none",
        Artefatti::Sorgente,
        true,
    ),
    descrittore(
        "io.write",
        "plenora-io-write-input-v1",
        "plenora-io-write-result-v1",
        CONTENT_TYPE_JSON,
        "local",
        Artefatti::SorgenteEDestinazione,
        true,
    ),
    descrittore(
        "io.convert",
        "plenora-io-convert-input-v1",
        "plenora-io-convert-v1",
        CONTENT_TYPE_JSON,
        "local",
        Artefatti::SorgenteEDestinazione,
        true,
    ),
];

/// La discovery della superficie, `plenora.io-tools#capabilities@1`
/// (registro comune `bindings/runtime-v1.json`, campo `discovery`).
///
/// Passa dalla stessa ammissione delle sei operazioni -- grammatica dei
/// metadati, identita' del risultato, scadenza, rifiuti prima
/// dell'invocazione -- e rende [`capacita`]. Non e' un'operazione del
/// catalogo, e il documento non la elenca fra le operazioni.
///
/// Il selettore e' `capabilities`, un solo segmento: e' il nome che il
/// registro comune da' all'entrypoint, mentre la grammatica delle operazioni
/// ne vuole almeno due. Il contratto d'ingresso e'
/// `plenora-io-catalog-query-v1`, l'oggetto vuoto di `io.catalog`: la
/// discovery descrive l'artefatto che risponde, e non c'e' niente da
/// chiedere. Le due scelte sono del binding finche' plenora-contracts non
/// fissa la richiesta di discovery.
static SCOPERTA: DescrittoreRuntime = DescrittoreRuntime {
    operazione: OPERAZIONE_SCOPERTA,
    versione: 1,
    contratto_ingresso: "plenora-io-catalog-query-v1",
    contratto_uscita: "plenora-capabilities-v2",
    content_type_uscita: CONTENT_TYPE_JSON,
    effetto: "none",
    artefatti: Artefatti::Nessuno,
    controlli: false,
};

/// Il selettore `plenora.capability.operation` della discovery.
pub const OPERAZIONE_SCOPERTA: &str = "capabilities";

/// Un'invocazione serializzata, come la consegna l'applicazione.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Invocazione {
    /// Il content type del payload, senza parametri.
    pub content_type: String,
    /// I metadati riservati della richiesta.
    pub metadata: MetadatiRichiesta,
    /// Il payload JSON dell'operazione, secondo il suo schema `*-input-v1`.
    ///
    /// Letto rifiutando le chiavi ripetute: `serde_json` le accetta tenendo
    /// l'ultima, e `{"source": "a", "source": "b"}` sarebbe diventato in
    /// silenzio una lettura di `b`.
    #[serde(deserialize_with = "senza_chiavi_ripetute")]
    pub payload: Value,
}

impl Invocazione {
    /// Legge un'invocazione serializzata, o rende l'errore `plenora-error-v1`
    /// del rifiuto.
    ///
    /// E' la via da preferire alla deserializzazione diretta: l'errore di
    /// `serde_json` porta nomi di campo e valori dell'ingresso, e qui diventa
    /// un errore curato senza dati, `protocol` e `remote_effect: none`
    /// (RT-011). Per eseguire un'invocazione arrivata come byte la via e'
    /// [`BindingRuntime::invoca_json`], che rende anche questo rifiuto come
    /// risultato, con i metadati della richiesta che si possono riflettere.
    ///
    /// # Errors
    ///
    /// Il documento `plenora-error-v1` se i byte non sono un'invocazione.
    pub fn da_json(byte: &[u8]) -> Result<Self, Value> {
        serde_json::from_slice(byte).map_err(|_| {
            local_err_doc(
                "RUNTIME_INVOCATION_INVALID",
                ErrorCategory::Protocol,
                ErrorPhase::Validate,
                &PublicMessage::Curated(
                    "l'invocazione runtime non e' un documento con content_type, metadata \
                     e payload nella forma del binding",
                ),
            )
            .1["error"]
                .clone()
        })
    }
}

/// I metadati riservati di una richiesta (RUNTIME-BINDING-1.0 §3 e §4).
///
/// Le chiavi facoltative assenti restano assenti; presenti con `null` sono
/// **rifiutate**, perche' leggere `null` come assenza farebbe partire senza
/// scadenza una richiesta che ne dichiarava una. Le chiavi che il binding 1.0
/// non riserva -- comprese le `plenora.*` sconosciute -- sono **ignorate**:
/// e' la regola dei membri facoltativi sconosciuti (RUNTIME-BINDING-1.0 §9),
/// e un controllo che il binding conosce e l'operazione non supporta si
/// rifiuta per RT-006, non qui.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MetadatiRichiesta {
    /// L'identita' del messaggio.
    #[serde(rename = "plenora.message.id")]
    pub id_messaggio: String,
    /// L'identita' del messaggio che ha causato questo, se c'e'.
    #[serde(
        rename = "plenora.message.causation_id",
        default,
        deserialize_with = "presente",
        skip_serializing_if = "Option::is_none"
    )]
    pub id_causa: Option<String>,
    /// La capacita' runtime: `plenora.io-tools`.
    #[serde(rename = "plenora.capability.name")]
    pub nome_capacita: String,
    /// La versione del binding, in cifre decimali canoniche.
    #[serde(rename = "plenora.capability.version")]
    pub versione_capacita: String,
    /// L'operazione: `io.read` e le altre.
    #[serde(rename = "plenora.capability.operation")]
    pub operazione: String,
    /// La versione dell'operazione, in cifre decimali canoniche.
    #[serde(rename = "plenora.operation.version")]
    pub versione_operazione: String,
    /// Il contratto d'ingresso dichiarato dal chiamante.
    #[serde(rename = "plenora.input.contract")]
    pub contratto_ingresso: String,
    /// La scadenza assoluta, RFC 3339 in UTC.
    #[serde(
        rename = "plenora.execution.deadline",
        default,
        deserialize_with = "presente",
        skip_serializing_if = "Option::is_none"
    )]
    pub scadenza: Option<String>,
    /// La chiave d'idempotenza: nessuna operazione la ammette, e presente e'
    /// rifiutata (RT-006).
    #[serde(
        rename = "plenora.execution.idempotency_key",
        default,
        deserialize_with = "presente",
        skip_serializing_if = "Option::is_none"
    )]
    pub chiave_idempotenza: Option<String>,
    /// La correlazione, che il risultato conserva.
    #[serde(rename = "plenora.trace.correlation_id")]
    pub id_correlazione: String,
}

/// I metadati di un risultato (RUNTIME-BINDING-1.0 §6 e §7).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
///
/// Che cosa si riflette dalla richiesta segue la regola R2 della matrice
/// comune del binding runtime (proposta, in attesa di ratifica in
/// plenora-contracts): un valore d'instradamento o d'identita' si copia byte
/// per byte **solo se canonico**; altrimenti la chiave si omette. Mai un
/// valore normalizzato, mai un segnaposto come `"0"` o `io.unknown`.
pub struct MetadatiRisultato {
    /// L'identita' di **questo** messaggio: sempre nuova, UUID v4 canonico
    /// (RUNTIME-BINDING-1.0 §3, «unique message identity»).
    #[serde(rename = "plenora.message.id")]
    pub id_messaggio: String,
    /// La causa diretta del risultato, cioe' il `plenora.message.id` della
    /// richiesta quando era canonico (proposta P della matrice, caso 6). La
    /// causazione della richiesta non si copia: affermerebbe una causa falsa.
    #[serde(
        rename = "plenora.message.causation_id",
        default,
        deserialize_with = "presente",
        skip_serializing_if = "Option::is_none"
    )]
    pub id_causa: Option<String>,
    /// L'operazione della richiesta, se scritta in forma canonica.
    #[serde(
        rename = "plenora.capability.operation",
        default,
        deserialize_with = "presente",
        skip_serializing_if = "Option::is_none"
    )]
    pub operazione: Option<String>,
    /// La versione dell'operazione della richiesta, se canonica.
    #[serde(
        rename = "plenora.operation.version",
        default,
        deserialize_with = "presente",
        skip_serializing_if = "Option::is_none"
    )]
    pub versione_operazione: Option<String>,
    /// Il contratto d'uscita, o `plenora-error-v1` per un errore.
    #[serde(rename = "plenora.output.contract")]
    pub contratto_uscita: String,
    /// La correlazione dell'invocazione (RT-012), se canonica.
    #[serde(
        rename = "plenora.trace.correlation_id",
        default,
        deserialize_with = "presente",
        skip_serializing_if = "Option::is_none"
    )]
    pub id_correlazione: Option<String>,
}

/// Il payload di un risultato.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Carico {
    /// Un documento JSON: il risultato di un'operazione che rende JSON, o un
    /// errore `plenora-error-v1`.
    Json(Value),
    /// Il flusso Arrow consegnato da `io.read`, con il documento
    /// `plenora-io-read-result-v1` che lo descrive: fedelta', perdite, righe,
    /// e il descrittore `delivered` dei byte.
    FlussoArrow {
        /// I byte del flusso, serializzazione `stream` di Arrow IPC.
        byte: Vec<u8>,
        /// Il documento del contratto d'uscita.
        rapporto: Value,
    },
}

/// Il risultato terminale di un'invocazione.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Risultato {
    /// Il content type del payload.
    pub content_type: String,
    /// I metadati, con la correlazione dell'invocazione.
    pub metadata: MetadatiRisultato,
    /// Il payload.
    pub payload: Carico,
}

impl Risultato {
    /// Se il risultato e' un errore terminale.
    #[must_use]
    pub fn e_un_errore(&self) -> bool {
        self.content_type == CONTENT_TYPE_ERRORE
    }
}

/// Perche' l'applicazione non risolve un riferimento.
///
/// Non porta testo: un messaggio scritto da chi risolve potrebbe contenere il
/// percorso a cui il riferimento puntava, e un errore pubblico non porta
/// percorsi ne' valori.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RifiutoArtefatto {
    /// Il riferimento non nomina un artefatto che esista.
    NonTrovato,
    /// Il chiamante non e' autorizzato a quell'artefatto.
    NonAutorizzato,
}

/// Il confine con cui l'applicazione autorizza e risolve gli artefatti.
///
/// I driver leggono file: un `GeoPackage` e' un database `SQLite`, uno
/// Shapefile e' un insieme di file accanto. Per questo il risolutore rende un
/// **percorso locale** materializzato dall'applicazione, e non un flusso. Il
/// percorso non attraversa il confine runtime: lo produce l'applicazione e lo
/// usa il componente, e nessun messaggio pubblico lo riporta (RT-013).
///
/// La sorgente deve portare il suffisso del suo formato quando l'operazione
/// riconosce il formato invece di riceverlo dichiarato (`io.inspect`,
/// `io.layers`, `io.read`), esattamente come sulla CLI.
pub trait RisolutoreArtefatti {
    /// Il percorso locale della sorgente nominata dal riferimento.
    ///
    /// # Errors
    ///
    /// [`RifiutoArtefatto`] se il riferimento non esiste o non e' autorizzato.
    fn sorgente(&self, riferimento: &str) -> Result<PathBuf, RifiutoArtefatto>;

    /// Il percorso locale in cui pubblicare la destinazione nominata.
    ///
    /// Il percorso non deve esistere: la pubblicazione e' atomica ed
    /// esclusiva, e una destinazione gia' presente e' rifiutata
    /// dall'operazione con `OUTPUT_EXISTS`, come sulla CLI.
    ///
    /// # Errors
    ///
    /// [`RifiutoArtefatto`] se il riferimento non e' autorizzato.
    fn destinazione(&self, riferimento: &str) -> Result<PathBuf, RifiutoArtefatto>;
}

/// Il binding, legato al risolutore dell'applicazione.
pub struct BindingRuntime<'a> {
    artefatti: &'a dyn RisolutoreArtefatti,
    orologio: fn() -> SystemTime,
    directory_temporanea: Option<PathBuf>,
}

impl<'a> BindingRuntime<'a> {
    /// Lega il binding al risolutore senza aprire nulla.
    #[must_use]
    pub fn new(artefatti: &'a dyn RisolutoreArtefatti) -> Self {
        Self {
            artefatti,
            orologio: SystemTime::now,
            directory_temporanea: None,
        }
    }

    /// L'orologio con cui si legge `plenora.execution.deadline`.
    ///
    /// Di default quello di sistema. Un'applicazione che ha gia' un orologio
    /// suo -- o una prova che non deve dipendere dal giorno in cui gira -- lo
    /// inietta qui: la scadenza e' un istante assoluto, e senza un orologio
    /// fissato il suo esito dipende dalla data.
    #[must_use]
    pub fn con_orologio(mut self, orologio: fn() -> SystemTime) -> Self {
        self.orologio = orologio;
        self
    }

    /// Dove `io.read` crea la directory privata della consegna.
    ///
    /// Di default la directory temporanea di sistema. La directory privata si
    /// crea e si rimuove dentro questa a ogni lettura, e un errore di
    /// rimozione e' un errore dell'operazione, non un residuo taciuto.
    #[must_use]
    pub fn con_directory_temporanea(mut self, genitore: impl Into<PathBuf>) -> Self {
        self.directory_temporanea = Some(genitore.into());
        self
    }

    /// Ammette ed esegue un'invocazione, e rende il risultato terminale.
    ///
    /// Non fallisce: un errore e' un [`Risultato`] con content type
    /// [`CONTENT_TYPE_ERRORE`] e un documento `plenora-error-v1`, che porta i
    /// quattro assi comuni (RT-010). Un'invocazione non ammessa fallisce prima
    /// di toccare il risolutore, con `remote_effect: none` (RT-011).
    #[must_use]
    pub fn invoca(&self, invocazione: &Invocazione, cancellazione: CancellationToken) -> Risultato {
        // La forma tipizzata passa dalla stessa ammissione dei byte: le regole
        // stanno in un posto solo.
        serde_json::to_value(invocazione).map_or_else(
            |_| risultato_di_errore(&Map::new(), errore_senza_forma()),
            |documento| self.invoca_valore(&documento, cancellazione),
        )
    }

    /// Ammette ed esegue un'invocazione arrivata come byte JSON.
    ///
    /// E' la via completa: un documento che non si legge, un metadato che non
    /// e' una stringa o che manca diventano un risultato `protocol` come gli
    /// altri rifiuti (RT-017), con i metadati della richiesta che si possono
    /// riflettere (RT-019). Le chiavi ripetute si rifiutano a ogni livello.
    #[must_use]
    pub fn invoca_json(&self, byte: &[u8], cancellazione: CancellationToken) -> Risultato {
        match serde_json::from_slice::<SenzaDoppioni>(byte) {
            Ok(SenzaDoppioni(documento)) => self.invoca_valore(&documento, cancellazione),
            Err(_) => risultato_di_errore(&Map::new(), errore_dell_invocazione()),
        }
    }

    fn invoca_valore(&self, documento: &Value, cancellazione: CancellationToken) -> Risultato {
        let vuoti = Map::new();
        let metadati = documento
            .get("metadata")
            .and_then(Value::as_object)
            .unwrap_or(&vuoti);
        match self.invoca_ammessa(documento, metadati, cancellazione) {
            Ok((descrittore, payload)) => Risultato {
                content_type: descrittore.content_type_uscita.to_owned(),
                metadata: MetadatiRisultato {
                    contratto_uscita: descrittore.contratto_uscita.to_owned(),
                    ..identita_del_risultato(metadati)
                },
                payload,
            },
            Err(errore) => risultato_di_errore(metadati, errore),
        }
    }

    fn invoca_ammessa(
        &self,
        documento: &Value,
        metadati: &Map<String, Value>,
        cancellazione: CancellationToken,
    ) -> Result<(&'static DescrittoreRuntime, Carico), Value> {
        let ammessa = ammetti(documento, metadati)?;
        let descrittore = ammessa.descrittore;
        // La scadenza diventa un `Instant` **adesso**, all'ammissione, e i
        // millisecondi rimasti si ricalcolano dopo il risolutore: il tempo che
        // l'applicazione impiega a materializzare gli artefatti consuma la
        // stessa scadenza, invece di aggiungersi a lei. Gia' passata e'
        // `timeout` (RT-018, terzo passo; RT-021).
        let scadenza = ammessa
            .scadenza
            .map(|istante| istante_della_scadenza(istante, (self.orologio)()))
            .transpose()?;
        let campi = campi_del_payload(descrittore, ammessa.payload)?;
        if scadenza.is_some() && campi.deadline_ms.is_some() {
            // RT-023: la scadenza sul runtime viaggia solo nei metadati, e un
            // payload che la porta anche lui si rifiuta, anche a valori uguali.
            return Err(local_err_doc(
                "RUNTIME_DEADLINE_TWICE",
                ErrorCategory::InvalidConfiguration,
                ErrorPhase::Validate,
                &PublicMessage::Curated(
                    "la scadenza arriva sia da plenora.execution.deadline sia da deadline_ms: \
                     su questa superficie viaggia soltanto nei metadati",
                ),
            )
            .1["error"]
                .clone());
        }
        let mut richiesta = self.richiesta(descrittore, &campi)?;
        let rimasti = scadenza.map(millisecondi_rimasti).transpose()?;
        richiesta.limits = tetti(&campi, rimasti);
        // La scadenza assoluta va nel **token**: quando arriva durante
        // l'esecuzione, la pipeline la osserva come `timeout`
        // (PUBLIC-SURFACES-1.0 SURF-010), non come una quota di risorse
        // esaurita. Il tetto di durata qui sopra resta oltre la scadenza, cosi'
        // da non arrivare prima di lei.
        richiesta = richiesta.con_cancellazione(match scadenza {
            Some(istante) => cancellazione.child_token_with_deadline(Some(istante)),
            None => cancellazione,
        });
        let esito = match descrittore.operazione {
            "io.catalog" => return Ok((descrittore, Carico::Json(operazioni::catalog()))),
            OPERAZIONE_SCOPERTA => return Ok((descrittore, Carico::Json(capacita()))),
            "io.inspect" => operazioni::inspect(richiesta),
            "io.layers" => operazioni::layers(richiesta),
            "io.read" => {
                return consegna_in_flusso(richiesta, self.directory_temporanea.as_deref())
                    .map(|carico| (descrittore, carico))
            }
            "io.write" => operazioni::write(richiesta),
            "io.convert" => operazioni::convert(richiesta),
            _ => return Err(errore_di_instradamento(RifiutoDiInstradamento::Operazione)),
        };
        esito
            .map(|documento| (descrittore, Carico::Json(documento)))
            .map_err(errore_della_busta)
    }

    /// La [`Richiesta`] dai campi del payload, con gli artefatti risolti.
    fn richiesta(
        &self,
        descrittore: &DescrittoreRuntime,
        campi: &Campi,
    ) -> Result<Richiesta, Value> {
        let mut richiesta = Richiesta::default();
        if descrittore.artefatti == Artefatti::Nessuno {
            return Ok(richiesta);
        }
        // Prima si guardano **tutti** i riferimenti, poi si chiede al
        // risolutore: un riferimento con la forma di un percorso non deve
        // raggiungere l'applicazione nemmeno quando l'altro e' valido.
        let sorgente = campi
            .source
            .as_deref()
            .map(riferimento_ammesso)
            .transpose()?;
        let destinazione = campi
            .destination
            .as_deref()
            .map(riferimento_ammesso)
            .transpose()?;
        if let Some(riferimento) = sorgente {
            let percorso = self
                .artefatti
                .sorgente(riferimento)
                .map_err(errore_del_risolutore)?;
            richiesta.source = Some(nome_coerente(riferimento, percorso)?);
        }
        if let Some(riferimento) = destinazione {
            let percorso = self
                .artefatti
                .destinazione(riferimento)
                .map_err(errore_del_risolutore)?;
            richiesta.destination = Some(nome_coerente(riferimento, percorso)?);
        }
        // `limit` non si trasferisce: lo schema lo ammette soltanto in
        // `io.read`, e su questa superficie `io.read` lo rifiuta prima di
        // arrivare qui (`consegna_runtime_ammissibile`).
        richiesta.layer = campi.layer;
        richiesta.assume_crs.clone_from(&campi.assume_crs);
        richiesta.durable = campi.durable.unwrap_or(false);
        richiesta.options = campi.options.clone().unwrap_or_default();
        richiesta.input_options = campi.input_options.clone().unwrap_or_default();
        richiesta.output_options = campi.output_options.clone().unwrap_or_default();
        richiesta.source_format.clone_from(&campi.source_format);
        richiesta.target_format = campi.target_format.clone().or_else(|| campi.format.clone());
        Ok(richiesta)
    }
}

/// `io.read`: consegna in una directory privata, e rende i byte.
///
/// La consegna e' la stessa della CLI -- staging, validazione fino a EOF,
/// pubblicazione atomica -- con la serializzazione `stream` imposta. Cambia
/// soltanto dove finisce: in una directory privata che questo processo crea e
/// rimuove, invece che in un percorso del chiamante.
///
/// La rimozione si verifica su **ogni** percorso, riuscito o fallito. Se non
/// riesce, l'errore e' quello della rimozione anche quando l'operazione era
/// fallita per conto suo: un file rimasto su disco e' l'effetto che il
/// descrittore runtime dichiara di non avere, ed e' la cosa che l'applicazione
/// deve sapere per prima.
fn consegna_in_flusso(richiesta: Richiesta, genitore: Option<&Path>) -> Result<Carico, Value> {
    let mut costruttore = tempfile::Builder::new();
    costruttore.prefix("plenora-io-consegna-");
    let privata = genitore
        .map_or_else(
            || costruttore.tempdir(),
            |genitore| costruttore.tempdir_in(genitore),
        )
        .map_err(|_| errore_della_consegna(Consegna::Apertura))?;
    let file = privata.path().join("consegna.arrows");
    let richiesta = richiesta
        .con_destinazione(&file)
        .con_opzione_di_scrittura("serialization", "stream");
    let esito = operazioni::read(richiesta)
        .map_err(errore_della_busta)
        .and_then(|rapporto| {
            std::fs::read(&file)
                .map(|byte| Carico::FlussoArrow { byte, rapporto })
                .map_err(|_| errore_della_consegna(Consegna::Lettura))
        });
    let rimossa = privata.close();
    match (esito, rimossa) {
        (_, Err(_)) => Err(errore_della_consegna(Consegna::Rimozione)),
        (esito, Ok(())) => esito,
    }
}

fn tetti(campi: &Campi, scadenza_ms: Option<u64>) -> PipelineLimits {
    let mut tetti = PipelineLimits::default();
    if let Some(budget) = &campi.budgets {
        // La stessa traduzione dei flag `--max-*` della CLI, quota per quota:
        // zero e incoerenze le rifiuta `PipelineLimits::validate` quando la
        // pipeline si costruisce, non questo binding.
        if let Some(valore) = budget.memory_bytes {
            tetti = tetti.with_memory_bytes(valore);
        }
        if let Some(valore) = budget.max_rows {
            tetti = tetti.with_max_rows(valore);
        }
        if let Some(valore) = budget.max_columns {
            tetti = tetti.with_max_columns(valore);
        }
        if let Some(valore) = budget.max_input_bytes {
            tetti = tetti.with_max_input_bytes(valore);
        }
        if let Some(valore) = budget.max_input_entries {
            tetti = tetti.with_max_input_entries(valore);
        }
        if let Some(valore) = budget.max_output_bytes {
            tetti = tetti.with_max_output_bytes(valore);
        }
        if let Some(valore) = budget.max_vertices {
            tetti = tetti.with_max_vertices(valore);
        }
        if let Some(valore) = budget.max_wkb_cell_bytes {
            tetti = tetti.with_max_wkb_cell_bytes(valore);
        }
        if let Some(valore) = budget.max_wkb_components {
            tetti = tetti.with_max_wkb_components(valore);
        }
        if let Some(valore) = budget.max_wkb_depth {
            tetti = tetti.with_max_wkb_depth(valore);
        }
    }
    // Una scadenza sola: la scadenza assoluta dei metadati **oppure**
    // `deadline_ms` del payload -- le due insieme sono rifiutate prima di
    // arrivare qui -- oppure il default del modello.
    //
    // Con la scadenza assoluta la governa il token: la durata della pipeline
    // sta un secondo oltre, perche' la pipeline nasce un poco dopo questo
    // calcolo e il suo limite non deve arrivare prima.
    let durata = scadenza_ms
        .map(|assoluta| assoluta.saturating_add(MARGINE_DELLA_DURATA_MS))
        .or(campi.deadline_ms);
    durata.map_or(tetti, |millisecondi| tetti.with_duration_ms(millisecondi))
}

/// I campi che gli schemi `*-input-v1` dichiarano, tutti insieme.
///
/// Quali valgono per quale operazione lo decide [`campi_ammessi`], prima che
/// il payload diventi questa struct: le sei condividono la forma dei campi
/// come condividono la [`Richiesta`].
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Campi {
    #[serde(default, deserialize_with = "presente")]
    source: Option<String>,
    #[serde(default, deserialize_with = "presente")]
    destination: Option<String>,
    #[serde(default, deserialize_with = "presente")]
    format: Option<String>,
    #[serde(default, deserialize_with = "presente")]
    source_format: Option<String>,
    #[serde(default, deserialize_with = "presente")]
    target_format: Option<String>,
    #[serde(default, deserialize_with = "presente")]
    layer: Option<u32>,
    #[serde(default, deserialize_with = "presente")]
    limit: Option<u64>,
    #[serde(default, deserialize_with = "presente")]
    assume_crs: Option<String>,
    #[serde(default, deserialize_with = "presente")]
    durable: Option<bool>,
    #[serde(default, deserialize_with = "presente")]
    options: Option<BTreeMap<String, String>>,
    #[serde(default, deserialize_with = "presente")]
    input_options: Option<BTreeMap<String, String>>,
    #[serde(default, deserialize_with = "presente")]
    output_options: Option<BTreeMap<String, String>>,
    #[serde(default, deserialize_with = "presente")]
    deadline_ms: Option<u64>,
    #[serde(default, deserialize_with = "presente")]
    budgets: Option<Budget>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Budget {
    #[serde(default, deserialize_with = "presente")]
    memory_bytes: Option<u64>,
    #[serde(default, deserialize_with = "presente")]
    max_rows: Option<u64>,
    #[serde(default, deserialize_with = "presente")]
    max_columns: Option<u64>,
    #[serde(default, deserialize_with = "presente")]
    max_input_bytes: Option<u64>,
    #[serde(default, deserialize_with = "presente")]
    max_input_entries: Option<u64>,
    #[serde(default, deserialize_with = "presente")]
    max_output_bytes: Option<u64>,
    #[serde(default, deserialize_with = "presente")]
    max_vertices: Option<usize>,
    #[serde(default, deserialize_with = "presente")]
    max_wkb_cell_bytes: Option<usize>,
    #[serde(default, deserialize_with = "presente")]
    max_wkb_components: Option<usize>,
    #[serde(default, deserialize_with = "presente")]
    max_wkb_depth: Option<usize>,
}

/// Un campo facoltativo presente e' un valore, mai `null`.
///
/// Senza questa funzione serde legge `"layer": null` come `None`, cioe' come
/// un campo omesso. Gli schemi non ammettono `null` in nessuno di questi
/// campi, e una scadenza o un tetto scritti `null` partirebbero col default.
fn presente<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// I campi ammessi e quelli obbligatori di ciascuna operazione.
///
/// Sono gli `properties` e i `required` degli schemi `*-input-v1`. Le prove
/// li confrontano con gli schemi pubblicati, chiave per chiave: una copia che
/// divergesse dallo schema farebbe del binding un contratto diverso.
#[must_use]
pub fn campi_ammessi(operazione: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match operazione {
        "io.inspect" | "io.layers" => (
            &["source", "assume_crs", "input_options", "deadline_ms"],
            &["source"],
        ),
        "io.read" => (
            &[
                "source",
                "layer",
                "destination",
                "limit",
                "assume_crs",
                "input_options",
                "deadline_ms",
                "budgets",
                "output_options",
                "durable",
            ],
            &["source"],
        ),
        "io.write" => (
            &[
                "source",
                "destination",
                "format",
                "layer",
                "durable",
                "output_options",
                "input_options",
                "assume_crs",
                "deadline_ms",
                "budgets",
            ],
            &["source", "destination", "format"],
        ),
        "io.convert" => (
            &[
                "source",
                "destination",
                "source_format",
                "target_format",
                "layer",
                "assume_crs",
                "durable",
                "input_options",
                "output_options",
                "options",
                "deadline_ms",
                "budgets",
            ],
            &["source", "destination", "source_format", "target_format"],
        ),
        _ => (&[], &[]),
    }
}

fn campi_del_payload(descrittore: &DescrittoreRuntime, payload: &Value) -> Result<Campi, Value> {
    let Value::Object(oggetto) = payload else {
        return Err(errore_del_payload(&PublicMessage::Curated(
            "il payload runtime e' un oggetto JSON",
        )));
    };
    let (ammessi, obbligatori) = campi_ammessi(descrittore.operazione);
    if oggetto
        .keys()
        .any(|chiave| !ammessi.contains(&chiave.as_str()))
    {
        return Err(errore_del_payload(&PublicMessage::Curated(
            "il payload runtime porta un campo che lo schema d'ingresso dell'operazione non dichiara",
        )));
    }
    if obbligatori
        .iter()
        .any(|chiave| !oggetto.contains_key(*chiave))
    {
        return Err(errore_del_payload(&PublicMessage::Curated(
            "il payload runtime non porta un campo che lo schema d'ingresso dell'operazione richiede",
        )));
    }
    let campi: Campi = serde_json::from_value(payload.clone()).map_err(|_| {
        errore_del_payload(&PublicMessage::Curated(
            "il payload runtime non rispetta i tipi dello schema d'ingresso dell'operazione",
        ))
    })?;
    vincoli_dello_schema(&campi)?;
    if descrittore.operazione == "io.read" {
        consegna_runtime_ammissibile(&campi)?;
    }
    Ok(campi)
}

/// I vincoli degli schemi che la forma dei tipi non esprime.
///
/// `minLength: 1` sulle stringhe, `minimum: 1` su `deadline_ms` e su
/// `memory_bytes`. Il resto -- un tetto a zero, due tetti incoerenti -- lo
/// rifiuta il modello quando la pipeline si costruisce, come per la CLI.
fn vincoli_dello_schema(campi: &Campi) -> Result<(), Value> {
    let stringhe = [
        &campi.source,
        &campi.destination,
        &campi.format,
        &campi.source_format,
        &campi.target_format,
        &campi.assume_crs,
    ];
    let vuota = stringhe
        .iter()
        .any(|campo| campo.as_deref().is_some_and(str::is_empty));
    let zero = campi.deadline_ms == Some(0)
        || campi
            .budgets
            .as_ref()
            .is_some_and(|budget| budget.memory_bytes == Some(0));
    if vuota || zero {
        return Err(errore_del_payload(&PublicMessage::Curated(
            "il payload runtime porta una stringa vuota o una quota a zero dove lo schema le vieta",
        )));
    }
    // L'enum dei formati di `io.write` e `io.convert`, prima del risolutore:
    // lo stesso rifiuto della CLI (`UNKNOWN_FORMAT`, dalla stessa funzione),
    // ma senza chiedere all'applicazione di materializzare nulla.
    for formato in [&campi.format, &campi.source_format, &campi.target_format]
        .into_iter()
        .flatten()
    {
        crate::driver_per_formato(formato).map_err(|(_, busta)| errore_della_busta(busta))?;
    }
    Ok(())
}

/// Su questa superficie `io.read` consegna sempre, e sempre nel risultato.
///
/// `destination`, `output_options` e `durable` descrivono un file del
/// chiamante, che qui non c'e': accettarli vorrebbe dire scrivere dove il
/// descrittore runtime dichiara di non scrivere. `limit` e' escluso per la
/// politica D9 dello schema, che vale per ogni consegna: `io.read` non
/// consegna dataset parziali.
fn consegna_runtime_ammissibile(campi: &Campi) -> Result<(), Value> {
    if campi.destination.is_some() || campi.output_options.is_some() || campi.durable.is_some() {
        return Err(local_err_doc(
            "RUNTIME_READ_DESTINATION",
            ErrorCategory::InvalidConfiguration,
            ErrorPhase::Validate,
            &PublicMessage::Curated(
                "sulla superficie runtime io.read consegna il flusso Arrow nel risultato: \
                 destination, output_options e durable descrivono un file che qui non esiste",
            ),
        )
        .1["error"]
            .clone());
    }
    if campi.limit.is_some() {
        return Err(local_err_doc(
            "LIMIT_WITH_DELIVERY",
            ErrorCategory::InvalidPlan,
            ErrorPhase::Validate,
            &PublicMessage::Curated(
                "sulla superficie runtime io.read consegna sempre, e limit con una consegna \
                 non e' ammesso: io.read non consegna dataset parziali",
            ),
        )
        .1["error"]
            .clone());
    }
    Ok(())
}

/// Un riferimento opaco ad artefatto, o un rifiuto prima del risolutore.
///
/// La forma e' quella che `plenora-data-execution-input-v3` fissa per i
/// propri riferimenti: uno schema minuscolo di almeno due caratteri seguito da
/// `:`, niente spazi ne' barre rovesciate, niente `file:`, niente segmenti
/// `.` o `..`, niente punti codificati. Un percorso locale -- `C:\dati`,
/// `/dati`, `../dati` -- non ha questa forma, e non attraversa il confine
/// (RT-013).
fn riferimento_ammesso(riferimento: &str) -> Result<&str, Value> {
    if riferimento_opaco(riferimento) {
        Ok(riferimento)
    } else {
        Err(local_err_doc(
            "RUNTIME_ARTIFACT_REFERENCE_INVALID",
            ErrorCategory::InvalidConfiguration,
            ErrorPhase::Validate,
            &PublicMessage::Curated(
                "un artefatto si nomina con un riferimento opaco, non con un percorso locale",
            ),
        )
        .1["error"]
            .clone())
    }
}

/// La forma di un riferimento opaco. Pubblica perche' l'applicazione possa
/// rifiutare in anticipo cio' che il binding rifiuterebbe.
#[must_use]
pub fn riferimento_opaco(riferimento: &str) -> bool {
    // `maxLength` di JSON Schema conta i caratteri, non i byte.
    if riferimento.chars().count() > MAX_CARATTERI_RIFERIMENTO {
        return false;
    }
    let Some((schema, resto)) = riferimento.split_once(':') else {
        return false;
    };
    let schema_valido = (2..=32).contains(&schema.len())
        && schema
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase())
        && schema.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'+' | b'.' | b'-')
        });
    let corpo = resto.strip_prefix("//").unwrap_or(resto);
    let corpo_valido = !corpo.is_empty()
        && !corpo
            .chars()
            .any(|carattere| spazio_ecma(carattere) || carattere == '\\');
    let minuscolo = riferimento.to_ascii_lowercase();
    let segmento_punto = resto
        .split(['/', ':'])
        .any(|segmento| segmento == "." || segmento == "..");
    schema_valido
        && corpo_valido
        && !minuscolo.starts_with("file:")
        && !minuscolo.contains("%2e")
        && !segmento_punto
}

/// Cio' che l'ammissione lascia passare all'esecuzione.
struct Ammessa<'a> {
    descrittore: &'static DescrittoreRuntime,
    scadenza: Option<&'a str>,
    payload: &'a Value,
}

/// Le chiavi riservate obbligatorie della richiesta (RUNTIME-BINDING-1.0 §3).
const OBBLIGATORIE: [&str; 7] = [
    "plenora.message.id",
    "plenora.trace.correlation_id",
    "plenora.capability.name",
    "plenora.capability.version",
    "plenora.capability.operation",
    "plenora.operation.version",
    "plenora.input.contract",
];

/// L'ammissione, nell'ordine di RT-018: prima ogni chiave riservata nella
/// sua grammatica (`protocol`, RT-017), poi il supporto (`unsupported`,
/// RT-004/005/006/011), infine la scadenza gia' passata (`timeout`, che
/// calcola il chiamante con l'orologio del binding). Tutto prima
/// dell'invocazione: `validate`, `none`, `never` (RT-016).
fn ammetti<'a>(
    documento: &'a Value,
    metadati: &'a Map<String, Value>,
) -> Result<Ammessa<'a>, Value> {
    let Value::Object(radice) = documento else {
        return Err(errore_dell_invocazione());
    };
    let content_type = radice.get("content_type").and_then(Value::as_str);
    let forma_dell_invocazione = radice
        .keys()
        .all(|chiave| matches!(chiave.as_str(), "content_type" | "metadata" | "payload"));
    let (Some(content_type), Some(payload), true, true) = (
        content_type,
        radice.get("payload"),
        radice.get("metadata").is_some_and(Value::is_object),
        forma_dell_invocazione,
    ) else {
        return Err(errore_dell_invocazione());
    };

    // RT-017: obbligatorie presenti e stringhe, ciascuna nella sua grammatica.
    let mut valori = [""; OBBLIGATORIE.len()];
    for (valore, chiave) in valori.iter_mut().zip(OBBLIGATORIE) {
        *valore = metadati
            .get(chiave)
            .and_then(Value::as_str)
            .ok_or_else(|| errore_di_instradamento(RifiutoDiInstradamento::Malformato))?;
    }
    let [id_messaggio, id_correlazione, nome, versione_capacita, operazione, versione_operazione, contratto] =
        valori;
    let causa = facoltativa(metadati, "plenora.message.causation_id")?;
    if !uuid_canonico(id_messaggio)
        || !uuid_canonico(id_correlazione)
        || causa.is_some_and(|valore| !uuid_canonico(valore))
    {
        return Err(errore_di_instradamento(RifiutoDiInstradamento::Identita));
    }
    let instradamento = Instradamento {
        nome_capacita: nome,
        versione_capacita,
        operazione,
        versione_operazione,
        contratto_ingresso: contratto,
        content_type,
    };
    instradamento_ben_formato(&instradamento)?;
    let scadenza = facoltativa(metadati, "plenora.execution.deadline")?;
    if scadenza.is_some_and(|testo| istante_rfc3339_utc(testo).is_none()) {
        return Err(errore_della_scadenza_malformata());
    }
    // RT-022: presente, una chiave d'idempotenza e' una stringa non vuota
    // entro il limite; `null`, vuota o troppo lunga e' `protocol`.
    let chiave = facoltativa(metadati, "plenora.execution.idempotency_key")?;
    if chiave.is_some_and(|valore| valore.is_empty() || valore.len() > MAX_BYTE_CHIAVE_IDEMPOTENZA)
    {
        return Err(errore_di_instradamento(RifiutoDiInstradamento::Malformato));
    }

    // RT-018, secondo passo: tutto ben formato, ora il supporto.
    let descrittore = instradamento_supportato(&instradamento)?;
    if chiave.is_some() {
        return Err(errore_di_instradamento(RifiutoDiInstradamento::Idempotenza));
    }
    if scadenza.is_some() && !descrittore.controlli {
        return Err(errore_di_instradamento(RifiutoDiInstradamento::Scadenza));
    }
    Ok(Ammessa {
        descrittore,
        scadenza,
        payload,
    })
}

/// Una chiave riservata facoltativa: assente, o una stringa. Presente con un
/// altro tipo -- `null` compreso -- e' `protocol` (RT-017).
fn facoltativa<'a>(
    metadati: &'a Map<String, Value>,
    chiave: &str,
) -> Result<Option<&'a str>, Value> {
    metadati.get(chiave).map_or(Ok(None), |valore| {
        valore
            .as_str()
            .map(Some)
            .ok_or_else(|| errore_di_instradamento(RifiutoDiInstradamento::Malformato))
    })
}

/// Il limite di una chiave d'idempotenza, in byte: nessuna operazione di
/// IO-tools la ammette, e il limite serve soltanto a classificare un valore
/// fuori misura come malformato invece che come non supportato (RT-022).
const MAX_BYTE_CHIAVE_IDEMPOTENZA: usize = 256;

/// I metadati del risultato che si riflettono dalla richiesta (RT-019,
/// RT-020): un `message.id` nuovo, la causa e' il `message.id` della
/// richiesta, e operazione, versione e correlazione solo se ben formate,
/// byte per byte.
fn identita_del_risultato(metadati: &Map<String, Value>) -> MetadatiRisultato {
    let riflessa = |chiave: &str, forma: fn(&str) -> bool| {
        metadati
            .get(chiave)
            .and_then(Value::as_str)
            .filter(|valore| forma(valore))
            .map(str::to_owned)
    };
    MetadatiRisultato {
        id_messaggio: uuid::Uuid::new_v4().hyphenated().to_string(),
        id_causa: riflessa("plenora.message.id", uuid_canonico),
        operazione: riflessa("plenora.capability.operation", selettore_ben_formato),
        versione_operazione: riflessa("plenora.operation.version", versione_ben_formata),
        contratto_uscita: CONTRATTO_ERRORE.to_owned(),
        id_correlazione: riflessa("plenora.trace.correlation_id", uuid_canonico),
    }
}

fn risultato_di_errore(metadati: &Map<String, Value>, errore: Value) -> Risultato {
    Risultato {
        content_type: CONTENT_TYPE_ERRORE.to_owned(),
        metadata: identita_del_risultato(metadati),
        payload: Carico::Json(errore),
    }
}

fn errore_dell_invocazione() -> Value {
    local_err_doc(
        "RUNTIME_INVOCATION_INVALID",
        ErrorCategory::Protocol,
        ErrorPhase::Validate,
        &PublicMessage::Curated(
            "l'invocazione runtime non e' un documento con content_type, metadata e payload \
             nella forma del binding",
        ),
    )
    .1["error"]
        .clone()
}

fn errore_della_scadenza_malformata() -> Value {
    local_err_doc(
        "RUNTIME_DEADLINE_INVALID",
        ErrorCategory::Protocol,
        ErrorPhase::Validate,
        &PublicMessage::Curated(
            "plenora.execution.deadline e' un istante RFC 3339 in UTC: con Z o +00:00, mai \
             con uno scostamento diverso da zero o -00:00",
        ),
    )
    .1["error"]
        .clone()
}

/// I selettori d'instradamento di un'invocazione, presi in prestito.
#[derive(Clone, Copy, Debug)]
pub struct Instradamento<'a> {
    /// `plenora.capability.name`.
    pub nome_capacita: &'a str,
    /// `plenora.capability.version`.
    pub versione_capacita: &'a str,
    /// `plenora.capability.operation`.
    pub operazione: &'a str,
    /// `plenora.operation.version`.
    pub versione_operazione: &'a str,
    /// `plenora.input.contract`.
    pub contratto_ingresso: &'a str,
    /// Il content type del payload.
    pub content_type: &'a str,
}

/// Verifica l'instradamento prima di qualunque effetto (RT-004, RT-005, RT-011).
///
/// # Errors
///
/// Il documento `plenora-error-v1` del rifiuto, nell'ordine di RT-018:
/// `protocol` se un selettore non e' nella sua grammatica (RT-017), poi
/// `unsupported` se, tutti ben formati, non corrispondono a un'operazione
/// annunciata o il content type non e' annunciato. Sempre `validate`,
/// `remote_effect: none`, `retry: never` (RT-016). Un valore non si
/// normalizza mai: `"01"` non diventa `"1"`.
pub fn verifica_instradamento(
    instradamento: &Instradamento<'_>,
) -> Result<&'static DescrittoreRuntime, Value> {
    instradamento_ben_formato(instradamento)?;
    instradamento_supportato(instradamento)
}

/// RT-017 per i cinque selettori d'instradamento.
fn instradamento_ben_formato(instradamento: &Instradamento<'_>) -> Result<(), Value> {
    let ben_formato = capacita_ben_formata(instradamento.nome_capacita)
        && versione_ben_formata(instradamento.versione_capacita)
        && selettore_ben_formato(instradamento.operazione)
        && versione_ben_formata(instradamento.versione_operazione)
        && contratto_ben_formato(instradamento.contratto_ingresso);
    if ben_formato {
        Ok(())
    } else {
        Err(errore_di_instradamento(RifiutoDiInstradamento::Malformato))
    }
}

/// RT-018, secondo passo: selettori ben formati, ma annunciati?
fn instradamento_supportato(
    instradamento: &Instradamento<'_>,
) -> Result<&'static DescrittoreRuntime, Value> {
    use RifiutoDiInstradamento as R;
    if instradamento.nome_capacita != NOME_CAPACITA {
        return Err(errore_di_instradamento(R::NomeCapacita));
    }
    if !versione_annunciata(instradamento.versione_capacita, VERSIONE_BINDING) {
        return Err(errore_di_instradamento(R::VersioneCapacita));
    }
    let descrittore = OPERAZIONI
        .iter()
        .chain(std::iter::once(&SCOPERTA))
        .find(|voce| voce.operazione == instradamento.operazione)
        .ok_or_else(|| errore_di_instradamento(R::Operazione))?;
    if !versione_annunciata(instradamento.versione_operazione, descrittore.versione) {
        return Err(errore_di_instradamento(R::Versione));
    }
    if instradamento.contratto_ingresso != descrittore.contratto_ingresso {
        return Err(errore_di_instradamento(R::Contratto));
    }
    if instradamento.content_type != CONTENT_TYPE_JSON {
        return Err(errore_di_instradamento(R::ContentType));
    }
    Ok(descrittore)
}

/// Solo la forma decimale canonica, `^[1-9][0-9]*$`, di qualunque grandezza.
///
/// E' la grammatica (RT-017), non il supporto: `"4294967296"` e' ben formata
/// anche se non sta in un `u32`, e una versione ben formata che nessuno
/// annuncia e' `unsupported`, non `protocol` (RT-018). `u32::from_str`
/// accetterebbe anche `+1` e `01`, che differiscono dal descrittore come
/// testo e coincidono come numero.
fn versione_ben_formata(valore: &str) -> bool {
    !valore.is_empty()
        && !valore.starts_with('0')
        && valore.bytes().all(|byte| byte.is_ascii_digit())
}

/// Una versione ben formata che e' esattamente quella annunciata.
///
/// Il confronto e' sul testo: una versione oltre `u32` non e' un errore di
/// conversione, e' una versione diversa.
fn versione_annunciata(valore: &str, annunciata: u32) -> bool {
    versione_ben_formata(valore) && valore == annunciata.to_string()
}

/// `^plenora\.[a-z][a-z0-9-]*-tools$` (`runtime-vector-v1.schema.json`).
fn capacita_ben_formata(valore: &str) -> bool {
    valore
        .strip_prefix("plenora.")
        .and_then(|resto| resto.strip_suffix("-tools"))
        .is_some_and(|nome| {
            nome.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
                && nome
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// Un'operazione ben formata, o il selettore della discovery.
fn selettore_ben_formato(valore: &str) -> bool {
    valore == OPERAZIONE_SCOPERTA || operazione_ben_formata(valore)
}

/// `^[a-z][a-z0-9_-]*(\.[a-z][a-z0-9_-]*)+$` (`runtime-vector-v1.schema.json`).
fn operazione_ben_formata(valore: &str) -> bool {
    let segmenti: Vec<&str> = valore.split('.').collect();
    segmenti.len() >= 2
        && segmenti.iter().all(|segmento| {
            segmento
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_lowercase())
                && segmento
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        })
}

/// `^plenora-[a-z0-9-]+-v[1-9][0-9]*$` (`runtime-vector-v1.schema.json`).
fn contratto_ben_formato(valore: &str) -> bool {
    valore
        .strip_prefix("plenora-")
        .and_then(|resto| resto.rsplit_once("-v"))
        .is_some_and(|(nome, versione)| {
            !nome.is_empty()
                && nome
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && versione_ben_formata(versione)
        })
}

fn uuid_canonico(valore: &str) -> bool {
    valore.len() == 36
        && valore.bytes().enumerate().all(|(indice, byte)| {
            if matches!(indice, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

/// L'`Instant` della scadenza assoluta, letta con l'orologio del binding.
///
/// # Errors
///
/// Una scadenza che non e' nella grafia RFC 3339 UTC ammessa e' `protocol`.
/// Una scadenza gia' passata -- compresa una anteriore al 1970 -- e'
/// `timeout` (`errore_di_scadenza`). Un
/// orologio anteriore all'epoca Unix e' rifiutato: leggerlo come zero
/// allenterebbe in silenzio ogni scadenza.
fn istante_della_scadenza(testo: &str, adesso: SystemTime) -> Result<Instant, Value> {
    let scadenza = match istante_rfc3339_utc(testo) {
        Some(IstanteRfc3339::DallEpoca(durata)) => durata,
        Some(IstanteRfc3339::PrimaDellEpoca) => return Err(errore_di_scadenza()),
        None => return Err(errore_della_scadenza_malformata()),
    };
    let adesso = adesso.duration_since(UNIX_EPOCH).map_err(|_| {
        local_err_doc(
            "RUNTIME_CLOCK_INVALID",
            ErrorCategory::Internal,
            ErrorPhase::Validate,
            &PublicMessage::Curated(
                "l'orologio del binding e' anteriore all'epoca Unix: la scadenza non si puo' leggere",
            ),
        )
        .1["error"]
            .clone()
    })?;
    let restano = scadenza
        .checked_sub(adesso)
        .filter(|durata| !durata.is_zero())
        .ok_or_else(errore_di_scadenza)?;
    Instant::now().checked_add(restano).ok_or_else(|| {
        local_err_doc(
            "RUNTIME_DEADLINE_INVALID",
            ErrorCategory::InvalidConfiguration,
            ErrorPhase::Validate,
            &PublicMessage::Curated(
                "plenora.execution.deadline e' oltre l'orizzonte che il processo sa rappresentare",
            ),
        )
        .1["error"]
            .clone()
    })
}

/// I millisecondi interi che restano fino a `istante`, misurati adesso.
///
/// Arrotonda per **difetto**: mezzo millisecondo rimasto e' gia' una scadenza
/// passata, perche' la quota della pipeline si esprime in millisecondi interi
/// e arrotondare per eccesso la allungherebbe.
///
/// # Errors
///
/// [`errore_di_scadenza`] se non resta almeno un millisecondo.
fn millisecondi_rimasti(istante: Instant) -> Result<u64, Value> {
    let restano = istante
        .checked_duration_since(Instant::now())
        .map_or(0, |durata| {
            u64::try_from(durata.as_millis()).unwrap_or(u64::MAX)
        });
    if restano == 0 {
        return Err(errore_di_scadenza());
    }
    Ok(restano)
}

/// Una scadenza gia' passata all'ammissione.
///
/// `timeout`, come PUBLIC-SURFACES-1.0 SURF-010 vuole per ogni scadenza
/// osservata; `validate` e `none` perche' nulla e' iniziato; `never` perche'
/// lo stesso messaggio porta la stessa scadenza e fallirebbe di nuovo
/// (proposta P della matrice, caso 7b).
fn errore_di_scadenza() -> Value {
    local_err_doc(
        "DEADLINE_EXCEEDED",
        ErrorCategory::Timeout,
        ErrorPhase::Validate,
        &PublicMessage::Curated("la scadenza dell'invocazione e' gia' passata"),
    )
    .1["error"]
        .clone()
}

/// Un istante RFC 3339 in UTC, rispetto all'epoca Unix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IstanteRfc3339 {
    /// Un istante valido anteriore al 1970-01-01T00:00:00Z.
    PrimaDellEpoca,
    /// La durata dall'epoca Unix.
    DallEpoca(Duration),
}

/// Un istante RFC 3339 in UTC, o `None` se il testo non lo e'.
///
/// Le grafie RFC 3339 di un istante UTC: `AAAA-MM-GGTHH:MM:SS[.frazione]`
/// seguito da `Z`, `z` o `+00:00`, con `T` o `t`, e una frazione di quante
/// cifre si vuole. Uno scostamento diverso da zero e `-00:00` -- che RFC 3339
/// riserva all'ora locale sconosciuta -- non sono UTC, e rendono `None`
/// (RT-021).
///
/// Due grafie valide non hanno un nanosecondo Unix esatto, e si riconducono
/// **per eccesso**, al primo nanosecondo che non precede l'istante scritto:
///
/// * una frazione oltre le nove cifre, al nanosecondo successivo se le cifre
///   oltre la nona non sono tutte zero;
/// * il secondo intercalare `23:59:60[.f]`, all'inizio del giorno dopo, che e'
///   l'istante a cui il tempo Unix lo fa coincidere.
///
/// Per eccesso, e non per difetto, perche' l'istante serve a confrontarlo con
/// un orologio che conta nanosecondi interi: `scadenza <= adesso` vale per
/// l'istante scritto se e solo se vale per quello ricondotto, e una scadenza
/// non scade mai prima di quando il chiamante l'ha scritta. Il `60` si
/// ammette solo alle `23:59` (in UTC un secondo intercalare non cade altrove);
/// quali giorni ne abbiano uno davvero lo dice una tabella che cambia, e non
/// si verifica.
#[must_use]
pub fn istante_rfc3339_utc(testo: &str) -> Option<IstanteRfc3339> {
    let corpo = testo
        .strip_suffix('Z')
        .or_else(|| testo.strip_suffix('z'))
        .or_else(|| testo.strip_suffix("+00:00"))?;
    if corpo.len() < 19 {
        return None;
    }
    let cifre = |da: usize, a: usize| -> Option<u64> {
        let parte = corpo.get(da..a)?;
        parte
            .bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| parte.parse().ok())
            .flatten()
    };
    let separatori = corpo.as_bytes();
    if separatori.get(4) != Some(&b'-')
        || separatori.get(7) != Some(&b'-')
        || !matches!(separatori.get(10), Some(b'T' | b't'))
        || separatori.get(13) != Some(&b':')
        || separatori.get(16) != Some(&b':')
    {
        return None;
    }
    let (anno, mese, giorno) = (cifre(0, 4)?, cifre(5, 7)?, cifre(8, 10)?);
    let (ora, minuto, secondo) = (cifre(11, 13)?, cifre(14, 16)?, cifre(17, 19)?);
    let (nanosecondi, per_eccesso) = match corpo.get(19..) {
        Some("") => (0, false),
        Some(frazione) => {
            let cifre_frazione = frazione.strip_prefix('.')?;
            if cifre_frazione.is_empty() || !cifre_frazione.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let (nove, oltre) = cifre_frazione.split_at(cifre_frazione.len().min(9));
            let valore: u32 = nove.parse().ok()?;
            let scala = 10u32.checked_pow(u32::try_from(9 - nove.len()).ok()?)?;
            (valore.checked_mul(scala)?, oltre.bytes().any(|b| b != b'0'))
        }
        None => return None,
    };
    let intercalare = secondo == 60 && ora == 23 && minuto == 59;
    if !(1..=12).contains(&mese) || ora > 23 || minuto > 59 || (secondo > 59 && !intercalare) {
        return None;
    }
    let bisestile = (anno % 4 == 0 && anno % 100 != 0) || anno % 400 == 0;
    let giorni_del_mese = match mese {
        2 if bisestile => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if giorno == 0 || giorno > giorni_del_mese {
        return None;
    }
    // Validato prima di guardare l'anno: `1969-02-30` non e' un istante, e
    // dirlo «passato» lo farebbe sembrare tale.
    if anno < 1970 {
        return Some(IstanteRfc3339::PrimaDellEpoca);
    }
    let dall_epoca = giorni_dall_epoca(anno, mese, giorno)?
        .checked_mul(86_400)?
        .checked_add(ora * 3_600 + minuto * 60 + secondo)?;
    if intercalare {
        // `23:59:60` e' `dall_epoca` = inizio del giorno dopo; qualunque sua
        // frazione cade ancora dentro il secondo intercalare, che il tempo
        // Unix non ha: per eccesso resta l'inizio del giorno dopo.
        return Some(IstanteRfc3339::DallEpoca(Duration::from_secs(dall_epoca)));
    }
    let istante = Duration::new(dall_epoca, nanosecondi);
    Some(IstanteRfc3339::DallEpoca(if per_eccesso {
        istante.checked_add(Duration::from_nanos(1))?
    } else {
        istante
    }))
}

/// I giorni dal 1970-01-01 al giorno civile dato (calendario gregoriano).
///
/// L'algoritmo «days from civil» di Howard Hinnant, ristretto agli anni dal
/// 1970 in poi: niente aritmetica con segno, e i conteggi stanno in `u64`.
/// Il chiamante garantisce `anno >= 1970`, `1 <= mese <= 12` e `giorno >= 1`,
/// che e' cio' che rende le sottrazioni prive di trabocco.
const fn giorni_dall_epoca(anno: u64, mese: u64, giorno: u64) -> Option<u64> {
    let anno = if mese <= 2 { anno - 1 } else { anno };
    let era = anno / 400;
    let anno_dell_era = anno - era * 400;
    let mese_spostato = if mese > 2 { mese - 3 } else { mese + 9 };
    let giorno_dell_anno = (153 * mese_spostato + 2) / 5 + giorno - 1;
    let giorno_dell_era =
        anno_dell_era * 365 + anno_dell_era / 4 - anno_dell_era / 100 + giorno_dell_anno;
    (era * 146_097 + giorno_dell_era).checked_sub(719_468)
}

/// Il percorso risolto, se il suo nome coincide con quello del riferimento.
///
/// Diversi driver derivano il nome del layer dal nome del file -- `GeoJSON`,
/// CSV, KML, DXF, Shapefile, `GeoParquet`, Arrow IPC -- e quel nome esce nel
/// risultato e, con `io.write` e `io.convert`, dentro l'artefatto scritto. Sul
/// runtime il file e' materializzato dall'applicazione: un nome scelto da lei
/// (temporaneo, casuale, interno) uscirebbe nei risultati pubblici, e lo
/// stesso riferimento renderebbe risultati diversi secondo come l'applicazione
/// nomina i propri file.
///
/// La regola e' quindi che il nome del file, senza estensione, sia quello
/// dell'ultimo segmento del riferimento, senza estensione: cio' che esce e'
/// derivato soltanto da cio' che il chiamante ha scritto. Il percorso deve
/// anche essere rappresentabile come testo, perche' le operazioni lo leggono
/// cosi'; uno che non lo e' sarebbe stato scritto altrove.
fn nome_coerente(riferimento: &str, percorso: PathBuf) -> Result<PathBuf, Value> {
    let segmento = riferimento.rsplit(['/', ':']).next().unwrap_or(riferimento);
    let atteso = Path::new(segmento).file_stem();
    let trovato = percorso.file_stem();
    if percorso.to_str().is_some() && atteso.is_some() && atteso == trovato {
        return Ok(percorso);
    }
    Err(local_err_doc(
        "RUNTIME_ARTIFACT_NAME_MISMATCH",
        ErrorCategory::InvalidConfiguration,
        ErrorPhase::Prepare,
        &PublicMessage::Curated(
            "l'applicazione ha materializzato l'artefatto con un nome diverso da quello del \
             riferimento, o non rappresentabile come testo: il nome del file entra nei risultati",
        ),
    )
    .1["error"]
        .clone())
}

/// `\s` di ECMA-262, cioe' cio' che il `pattern` degli schemi comuni vieta.
///
/// Non `char::is_whitespace`, che segue la proprieta' Unicode `White_Space`:
/// ammette U+FEFF e rifiuta U+0085, e il binding accetterebbe un insieme di
/// riferimenti diverso da quello che lo schema dichiara.
const fn spazio_ecma(carattere: char) -> bool {
    matches!(
        carattere,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// Un JSON qualunque, con le chiavi ripetute rifiutate a ogni livello.
fn senza_chiavi_ripetute<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(ValoreSenzaDoppioni)
}

struct ValoreSenzaDoppioni;

impl<'de> Visitor<'de> for ValoreSenzaDoppioni {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("un valore JSON senza chiavi ripetute")
    }

    fn visit_bool<E>(self, valore: bool) -> Result<Value, E> {
        Ok(Value::Bool(valore))
    }

    fn visit_i64<E>(self, valore: i64) -> Result<Value, E> {
        Ok(Value::from(valore))
    }

    fn visit_u64<E>(self, valore: u64) -> Result<Value, E> {
        Ok(Value::from(valore))
    }

    fn visit_f64<E: serde::de::Error>(self, valore: f64) -> Result<Value, E> {
        // Un numero non finito non e' JSON: `null` qui sarebbe un valore
        // inventato, e si rifiuta invece di convertirlo.
        serde_json::Number::from_f64(valore)
            .map(Value::Number)
            .ok_or_else(|| serde::de::Error::custom("numero non rappresentabile"))
    }

    fn visit_str<E>(self, valore: &str) -> Result<Value, E> {
        Ok(Value::String(valore.to_owned()))
    }

    fn visit_string<E>(self, valore: String) -> Result<Value, E> {
        Ok(Value::String(valore))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        senza_chiavi_ripetute(deserializer)
    }

    fn visit_seq<A>(self, mut sequenza: A) -> Result<Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut elementi = Vec::new();
        while let Some(SenzaDoppioni(elemento)) = sequenza.next_element()? {
            elementi.push(elemento);
        }
        Ok(Value::Array(elementi))
    }

    fn visit_map<A>(self, mut mappa: A) -> Result<Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut campi = Map::new();
        while let Some((chiave, SenzaDoppioni(valore))) =
            mappa.next_entry::<String, SenzaDoppioni>()?
        {
            if campi.insert(chiave, valore).is_some() {
                return Err(serde::de::Error::custom("chiave ripetuta nel payload"));
            }
        }
        Ok(Value::Object(campi))
    }
}

struct SenzaDoppioni(Value);

impl<'de> Deserialize<'de> for SenzaDoppioni {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        senza_chiavi_ripetute(deserializer).map(SenzaDoppioni)
    }
}

#[derive(Clone, Copy)]
enum RifiutoDiInstradamento {
    Identita,
    Malformato,
    NomeCapacita,
    VersioneCapacita,
    Operazione,
    Versione,
    Contratto,
    ContentType,
    Idempotenza,
    Scadenza,
}

fn errore_di_instradamento(rifiuto: RifiutoDiInstradamento) -> Value {
    let (codice, categoria, messaggio) = match rifiuto {
        RifiutoDiInstradamento::Identita => (
            "RUNTIME_IDENTITY_INVALID",
            ErrorCategory::Protocol,
            "le identita' runtime sono UUID canonici, minuscoli e con i trattini",
        ),
        RifiutoDiInstradamento::Malformato => (
            "RUNTIME_ROUTE_INVALID",
            ErrorCategory::Protocol,
            "un selettore d'instradamento non e' nella forma canonica del binding",
        ),
        RifiutoDiInstradamento::NomeCapacita => (
            "RUNTIME_CAPABILITY_UNSUPPORTED",
            ErrorCategory::Unsupported,
            "la capacita' runtime non e' plenora.io-tools",
        ),
        // ERR-002: una versione non supportata e' `unsupported`.
        RifiutoDiInstradamento::VersioneCapacita => (
            "RUNTIME_BINDING_VERSION_UNSUPPORTED",
            ErrorCategory::Unsupported,
            "questo artefatto serve soltanto la versione 1 del binding runtime",
        ),
        RifiutoDiInstradamento::Operazione => (
            "RUNTIME_OPERATION_UNSUPPORTED",
            ErrorCategory::Unsupported,
            "l'operazione runtime non e' fra quelle che questo artefatto serve",
        ),
        RifiutoDiInstradamento::Versione => (
            "RUNTIME_OPERATION_UNSUPPORTED",
            ErrorCategory::Unsupported,
            "la versione dell'operazione runtime non e' fra quelle che questo artefatto serve",
        ),
        RifiutoDiInstradamento::Contratto => (
            "RUNTIME_INPUT_CONTRACT_UNSUPPORTED",
            ErrorCategory::Unsupported,
            "il contratto d'ingresso non e' quello che l'operazione annuncia",
        ),
        RifiutoDiInstradamento::ContentType => (
            "RUNTIME_CONTENT_TYPE_UNSUPPORTED",
            ErrorCategory::Unsupported,
            "il content type non e' fra quelli che l'operazione annuncia",
        ),
        RifiutoDiInstradamento::Idempotenza => (
            "RUNTIME_CONTROL_UNSUPPORTED",
            ErrorCategory::Unsupported,
            "nessuna operazione di io-tools ammette una chiave d'idempotenza",
        ),
        RifiutoDiInstradamento::Scadenza => (
            "RUNTIME_CONTROL_UNSUPPORTED",
            ErrorCategory::Unsupported,
            "io.catalog non ammette una scadenza: non apre nulla e non la osserverebbe",
        ),
    };
    local_err_doc(
        codice,
        categoria,
        ErrorPhase::Validate,
        &PublicMessage::Curated(messaggio),
    )
    .1["error"]
        .clone()
}

fn errore_del_payload(messaggio: &PublicMessage) -> Value {
    local_err_doc(
        "RUNTIME_PAYLOAD_INVALID",
        ErrorCategory::InvalidConfiguration,
        ErrorPhase::Validate,
        messaggio,
    )
    .1["error"]
        .clone()
}

fn errore_del_risolutore(rifiuto: RifiutoArtefatto) -> Value {
    let (codice, categoria, messaggio) = match rifiuto {
        RifiutoArtefatto::NonTrovato => (
            "RUNTIME_ARTIFACT_NOT_FOUND",
            ErrorCategory::NotFound,
            "l'applicazione non risolve il riferimento ad artefatto",
        ),
        RifiutoArtefatto::NonAutorizzato => (
            "RUNTIME_ARTIFACT_UNAUTHORIZED",
            ErrorCategory::Authorization,
            "l'applicazione non autorizza il riferimento ad artefatto",
        ),
    };
    local_err_doc(
        codice,
        categoria,
        ErrorPhase::Prepare,
        &PublicMessage::Curated(messaggio),
    )
    .1["error"]
        .clone()
}

/// Dove la consegna di `io.read` si e' fermata.
#[derive(Clone, Copy)]
enum Consegna {
    Apertura,
    Lettura,
    Rimozione,
}

fn errore_della_consegna(passo: Consegna) -> Value {
    let (fase, messaggio) = match passo {
        Consegna::Apertura => (
            ErrorPhase::Prepare,
            "la directory privata della consegna non si e' potuta creare",
        ),
        Consegna::Lettura => (
            ErrorPhase::Commit,
            "il flusso Arrow consegnato non si e' potuto leggere dalla directory privata",
        ),
        Consegna::Rimozione => (
            ErrorPhase::Cleanup,
            "la directory privata della consegna non si e' potuta rimuovere: puo' restare un \
             file temporaneo",
        ),
    };
    local_err_doc(
        "RUNTIME_DELIVERY_FAILED",
        ErrorCategory::Io,
        fase,
        &PublicMessage::Curated(messaggio),
    )
    .1["error"]
        .clone()
}

/// L'errore della busta della CLI, cioe' l'oggetto `plenora-error-v1`.
///
/// Le operazioni rendono la busta del protocollo CLI, che avvolge l'errore
/// comune con `status`, `protocol_version` e `contract`: quei campi sono del
/// binding di processo, e il binding runtime porta soltanto l'errore.
fn errore_della_busta(busta: Value) -> Value {
    match busta {
        Value::Object(mut campi) if campi.get("error").is_some_and(Value::is_object) => {
            campi.remove("error").unwrap_or_else(errore_senza_forma)
        }
        // Ogni busta d'errore delle operazioni porta `error`: se non lo
        // porta, non si inventa un errore plausibile ma si dichiara quello
        // vero, che e' interno.
        _ => errore_senza_forma(),
    }
}

fn errore_senza_forma() -> Value {
    local_err_doc(
        "RUNTIME_ERROR_ENVELOPE_INVALID",
        ErrorCategory::Internal,
        ErrorPhase::Validate,
        &PublicMessage::Curated("l'operazione ha reso un errore senza la forma plenora-error-v1"),
    )
    .1["error"]
        .clone()
}

/// Il documento capability della superficie runtime (`plenora.io-tools#capabilities@1`).
///
/// Descrive **questa** superficie, come il documento della CLI descrive la
/// propria: una sola interfaccia, e le sei operazioni con ciò che qui
/// accettano e rendono. `io.read` vi dichiara `side_effect: none` e il solo
/// flusso Arrow in uscita, perche' qui consegna nel risultato; `io.write`
/// accetta il solo payload JSON, perche' il dataset arriva per riferimento.
#[must_use]
pub fn capacita() -> Value {
    let operazioni: Vec<Value> = OPERAZIONI
        .iter()
        .map(|voce| {
            // Niente `interchange_contracts`: il payload di `capabilities-v2`
            // e' chiuso, e il contratto d'interscambio lo dice il catalogo.
            let uscita = json!({
                "contract": voce.contratto_uscita,
                "content_types": [voce.content_type_uscita],
            });
            let mut operazione = json!({
                "id": voce.operazione,
                "version": voce.versione,
                "status": "available",
                "surfaces": ["runtime"],
                "input": {"contract": voce.contratto_ingresso, "content_types": [CONTENT_TYPE_JSON]},
                "output": uscita,
                "side_effect": voce.effetto,
                "controls": {
                    "cancellation": voce.controlli,
                    "deadline": voce.controlli,
                    "idempotency_key": false,
                },
            });
            if voce.operazione == "io.read" {
                operazione["attributes"] = json!({
                    "materialization": "bounded",
                    "delivery": "operation_atomic",
                });
            }
            operazione
        })
        .collect();
    json!({
        "schema_version": 2,
        "component": COMPONENTE,
        "component_version": env!("CARGO_PKG_VERSION"),
        "interfaces": [{
            "kind": "runtime",
            "contract": CONTRATTO_BINDING,
            "version": VERSIONE_BINDING,
            "artifact": NOME_CAPACITA,
        }],
        "operations": operazioni,
    })
}

#[cfg(test)]
mod sonde;
