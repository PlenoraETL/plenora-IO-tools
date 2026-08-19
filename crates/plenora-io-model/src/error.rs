use std::fmt;

use serde::{Deserialize, Serialize};

use crate::crs::RawCrs;
use crate::diagnostics::RowDiagnostics;

pub type Result<T> = std::result::Result<T, PlenoraIoError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityReason {
    EmptyWritePlan,
    MultipleLayers,
    DuplicateLayerName,
    FieldNameTooLong,
    FieldNameEncoding,
    FieldNameCollision,
    TypeNotRepresentable,
    GeometryNotSupported,
    MixedGeometry,
    GeometryEncoding,
    CoordinateDimensions,
    SpatialSemantics,
    CrsUnresolved,
    CrsRepresentationsInconsistent,
    ReprojectionRequired,
    Nullability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    InvalidPlan,
    /// La richiesta non e' ben formata per il driver scelto.
    ///
    /// Regola normativa (errata S6 del pacchetto decisionale del Lotto 0):
    /// chiave sconosciuta, fase errata, valore malformato o fuori dominio
    /// stanno **qui**; una richiesta valida che il driver o il formato non
    /// sanno servire sta in [`ErrorCategory::Unsupported`].
    ///
    /// La differenza non e' terminologica. `Unsupported` e' una risposta sul
    /// prodotto, e davanti a essa un chiamante automatico cambia driver o
    /// formato; questa e' una risposta sull'input, e la reazione corretta e'
    /// correggere la richiesta. Instradare un refuso verso `Unsupported`
    /// manda chi automatizza nella direzione sbagliata.
    InvalidConfiguration,
    Schema,
    DataMapping,
    Crs,
    /// La richiesta e' ben formata, ma il driver o il formato non hanno la
    /// capability che serve.
    ///
    /// L'altro lato della regola su [`ErrorCategory::InvalidConfiguration`]:
    /// qui non c'e' niente da correggere nella richiesta — si cambia driver,
    /// formato, o si rinuncia.
    Unsupported,
    NotFound,
    Conflict,
    Authentication,
    Authorization,
    Timeout,
    Cancelled,
    ResourceLimit,
    Io,
    Protocol,
    Transient,
    Execution,
    Internal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorPhase {
    Validate,
    Connect,
    Probe,
    Prepare,
    Read,
    Write,
    Finalize,
    Commit,
    Rollback,
    Cleanup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteEffect {
    None,
    RolledBack,
    Partial,
    Committed,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "delay_ms", rename_all = "snake_case")]
pub enum RetryDisposition {
    Never,
    Safe,
    RequiresIdempotencyKey,
    RequiresRecovery,
    After(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IoErrorCode {
    Generic,
    Contract,
    Unsupported,
    Capability,
    Schema,
    Format,
    Crs,
    CrsUnresolved,
    Wkb,
    LimitExceeded,
    ReaderBusy,
    ProjectionUnsupported,
    OutputExists,
    Io,
    Json,
    Cancelled,
}

impl IoErrorCode {
    /// Tutti i codici, in ordine di dichiarazione.
    ///
    /// Serve alla matrice di handoff, che deve elencare il vocabolario invece
    /// di ricopiarlo: un elenco copiato diverge alla prima variante aggiunta, e
    /// diverge in silenzio. Qui una variante nuova che non compaia in questo
    /// array e' un errore che il test della matrice prende.
    pub const TUTTI: &'static [Self] = &[
        Self::Generic,
        Self::Contract,
        Self::Unsupported,
        Self::Capability,
        Self::Schema,
        Self::Format,
        Self::Crs,
        Self::CrsUnresolved,
        Self::Wkb,
        Self::LimitExceeded,
        Self::ReaderBusy,
        Self::ProjectionUnsupported,
        Self::OutputExists,
        Self::Io,
        Self::Json,
        Self::Cancelled,
    ];
}

/// Errore pubblico redatto del bordo I/O.
///
/// I quattro assi sono indipendenti e serializzabili. `message` contiene solo
/// contesto operativo: mai payload, definizioni CRS, percorsi assoluti o valori
/// di cella.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlenoraIoError {
    pub code: IoErrorCode,
    pub category: ErrorCategory,
    pub phase: ErrorPhase,
    pub remote_effect: RemoteEffect,
    pub retry: RetryDisposition,
    pub driver: Option<String>,
    pub field: Option<String>,
    pub capability_reason: Option<CapabilityReason>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_diagnostics: Option<Box<RowDiagnostics>>,
}

/// L'identificatore di un elemento di un contratto **gia' validato**: il nome
/// di un campo o di un layer.
///
/// # Perche' e' sicuro, e perche' non e' un'eccezione
///
/// Non e' testo runtime arbitrario. Nasce solo risolvendo un contratto che il
/// prodotto ha gia' validato, e i nomi di quel contratto sono **gia' pubblici**:
/// gli envelope `inspect` e `layers` li emettono. Un identificatore qui non
/// apre un canale nuovo, nomina qualcosa che il chiamante ha gia' visto.
///
/// E' la ragione per cui INV-10 lo ammette pur vietando la `String` libera, ed
/// e' anche perche' **non** passa da [`PublicMessage`]: vive nel contesto
/// strutturato, e sara' il DTO a decidere se e dove emetterlo.
///
/// # Cosa lo distingue da `RowDiagnosticColumn`
///
/// `RowDiagnosticColumn::attest` accetta `impl Into<String>` — un costruttore
/// libero — perche' serve a un contratto diverso, con la propria policy
/// `emit`/`redact`. Qui non c'e' un costruttore libero: si parte da uno schema
/// o da un layer, e da nient'altro.
///
/// ```compile_fail
/// use plenora_io_model::ContractIdentifier;
/// let _ = ContractIdentifier::from("nome_arbitrario");
/// ```
///
/// ```compile_fail
/// use plenora_io_model::ContractIdentifier;
/// let _ = ContractIdentifier::new(String::from("nome_arbitrario"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractIdentifier {
    /// Il nome, gia' verificato non vuoto e di lunghezza ragionevole alla
    /// costruzione. Privato: non esiste un accessor che lo riporti fuori come
    /// `String` libera, solo `Display`.
    nome: String,
}

/// Tetto sul nome di un identificatore, in caratteri.
///
/// Gli schemi validati hanno nomi corti; il tetto e' una rete, non una regola
/// di dominio. Un nome oltre il tetto non produce un identificatore troncato
/// ma **nessun identificatore**: meglio non nominare che nominare a meta',
/// perche' un nome troncato somiglia a un nome vero.
const MAX_IDENTIFICATORE: usize = 256;

impl ContractIdentifier {
    /// L'identificatore di un campo, preso dallo schema che lo dichiara.
    ///
    /// `None` se l'indice non esiste nello schema o se il nome non e'
    /// nominabile — vuoto o oltre il tetto. Fallibile per costruzione: un
    /// indice fuori intervallo e' un difetto del chiamante, e restituirgli un
    /// identificatore inventato lo nasconderebbe.
    #[must_use]
    pub fn from_schema_field(
        schema: &arrow_schema::Schema,
        index: crate::contract::FieldId,
    ) -> Option<Self> {
        let posizione = usize::try_from(index.0).ok()?;
        let campo = schema.fields().get(posizione)?;
        Self::da_nome_validato(campo.name())
    }

    /// L'identificatore di un layer, preso dal contratto che lo dichiara.
    #[must_use]
    pub fn from_layer(layer: &crate::contract::LayerContract) -> Option<Self> {
        Self::da_nome_validato(&layer.name)
    }

    /// L'unica via interna: un nome che viene da un contratto validato.
    ///
    /// Privata di proposito. Se fosse pubblica, «viene da un contratto
    /// validato» tornerebbe a essere una promessa del chiamante invece di una
    /// proprieta' del tipo.
    fn da_nome_validato(nome: &str) -> Option<Self> {
        if nome.is_empty() || nome.chars().count() > MAX_IDENTIFICATORE {
            return None;
        }
        Some(Self {
            nome: nome.to_owned(),
        })
    }
}

impl std::fmt::Display for ContractIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.nome)
    }
}

/// Il contesto strutturato di un errore.
///
/// # Semantico, non del wire
///
/// Non ha campi che si chiamano come quelli di un contratto, non e'
/// serializzabile e non conosce nessun envelope. Dice **cosa** si sa
/// dell'errore — quale driver, quale layer, quale campo, quale ragione di
/// capability — e si ferma li'.
///
/// La traduzione verso `plenora-io-error-v1` e, in futuro, verso
/// `plenora-error-v1`, e' compito del DTO: e' **l'unico** adattatore, ed e' il
/// posto dove `driver` diventa `provider` e il resto confluisce in `details`.
/// Se quei nomi comparissero qui, il tipo semantico diventerebbe una copia del
/// wire e cambierebbe ogni volta che il wire cambia — che e' esattamente cio'
/// che tenerli separati evita.
///
/// # Niente testo libero
///
/// Nessun campo accetta testo da dipendenze o payload. L'unico testo e' il
/// nome di un [`ContractIdentifier`], che viene da un contratto validato.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ErrorContext {
    driver: Option<&'static str>,
    layer: Option<crate::contract::LayerId>,
    field: Option<crate::contract::FieldId>,
    identificatore: Option<ContractIdentifier>,
    capability_reason: Option<CapabilityReason>,
}

impl ErrorContext {
    /// Un contesto vuoto, da riempire con i metodi `con_*`.
    #[must_use]
    pub const fn nuovo() -> Self {
        Self {
            driver: None,
            layer: None,
            field: None,
            identificatore: None,
            capability_reason: None,
        }
    }

    #[must_use]
    pub const fn con_driver(mut self, driver: &'static str) -> Self {
        self.driver = Some(driver);
        self
    }

    #[must_use]
    pub const fn con_layer(mut self, layer: crate::contract::LayerId) -> Self {
        self.layer = Some(layer);
        self
    }

    #[must_use]
    pub const fn con_campo(mut self, field: crate::contract::FieldId) -> Self {
        self.field = Some(field);
        self
    }

    #[must_use]
    pub fn con_identificatore(mut self, identificatore: ContractIdentifier) -> Self {
        self.identificatore = Some(identificatore);
        self
    }

    #[must_use]
    pub const fn con_capability(mut self, reason: CapabilityReason) -> Self {
        self.capability_reason = Some(reason);
        self
    }

    #[must_use]
    pub const fn driver(&self) -> Option<&'static str> {
        self.driver
    }

    #[must_use]
    pub const fn layer(&self) -> Option<crate::contract::LayerId> {
        self.layer
    }

    #[must_use]
    pub const fn campo(&self) -> Option<crate::contract::FieldId> {
        self.field
    }

    #[must_use]
    pub const fn identificatore(&self) -> Option<&ContractIdentifier> {
        self.identificatore.as_ref()
    }

    #[must_use]
    pub const fn capability_reason(&self) -> Option<CapabilityReason> {
        self.capability_reason
    }
}

/// Un numero che si puo' far uscire in un messaggio d'errore.
///
/// La ratifica di S9 ammette «indici, conteggi, limiti o codici strutturali
/// tipizzati; mai valori numerici letti dal payload». Il tipo non puo'
/// verificare da dove viene un `u64` — nessun tipo puo' — ma **nomina il
/// ruolo** al sito di costruzione, e quello e' cio' che un tipo puo' fare qui:
/// scrivere `NumeroStrutturale::Valore(cella)` e' un gesto che si vede in
/// review, mentre passare un `u64` in mezzo ad altri no.
///
/// La distinzione non e' pedanteria. «riga 47» e' una posizione che il
/// chiamante conosce gia'; «valore 47.3» e' il dato. Il primo aiuta a trovare
/// il problema, il secondo lo espone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NumeroStrutturale {
    /// Posizione in una sequenza: indice di layer, di campo, di riga.
    Indice(u64),
    /// Quante cose sono state contate.
    Conteggio(u64),
    /// Il valore di una quota configurata.
    Limite(u64),
}

impl std::fmt::Display for NumeroStrutturale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Indice(n) | Self::Conteggio(n) | Self::Limite(n) => write!(f, "{n}"),
        }
    }
}

/// Il testo pubblico di un errore, deciso a compile time.
///
/// # Nessun testo runtime, e il compilatore lo impone
///
/// Una `String` non entra:
///
/// ```compile_fail
/// use plenora_io_model::PublicMessage;
/// let a_runtime = String::from("valore letto dal file");
/// let _ = PublicMessage::Curated(a_runtime);
/// ```
///
/// Nemmeno un `&str` preso in prestito da una `String`, che vivrebbe meno di
/// `'static`:
///
/// ```compile_fail
/// use plenora_io_model::PublicMessage;
/// fn da_runtime(valore: &str) -> PublicMessage {
///     PublicMessage::Curated(valore)
/// }
/// ```
///
/// Nemmeno il testo di una dipendenza, che e' il caso da cui tutto e' partito:
///
/// ```compile_fail
/// use plenora_io_model::PublicMessage;
/// fn da_dipendenza(errore: std::io::Error) -> PublicMessage {
///     PublicMessage::Curated(&errore.to_string())
/// }
/// ```
///
/// Un letterale invece si', anche in contesto `const`:
///
/// ```
/// use plenora_io_model::PublicMessage;
/// const MESSAGGIO: PublicMessage = PublicMessage::Curated("footer Parquet non valido");
/// assert_eq!(MESSAGGIO.to_string(), "footer Parquet non valido");
/// ```
///
/// E un numero strutturale, sempre in `const`:
///
/// ```
/// use plenora_io_model::{NumeroStrutturale, PublicMessage};
/// const OLTRE: PublicMessage = PublicMessage::CuratedWith(
///     "layer fuori dal piano di scrittura:",
///     NumeroStrutturale::Indice(3),
/// );
/// assert_eq!(OLTRE.to_string(), "layer fuori dal piano di scrittura: 3");
/// ```
///
/// # Perche' non e' una `String`
///
/// Una `String` alimentata a runtime e' un canale, e il canale era gia' usato:
/// 105 siti su 144 propagavano il testo d'errore di una dipendenza —
/// `calamine`, `parquet`, `arrow`, `csv`, `serde_json`, `rusqlite`, GDAL —
/// e nessuna di quelle librerie promette che il proprio messaggio non contenga
/// un percorso o un valore di cella. Per XLSX era gia' successo.
///
/// # La regola, e la sua unica eccezione
///
/// **Nessun testo runtime, salvo il token bounded di un'opzione rifiutata
/// prodotto dal validatore centrale.** L'eccezione e' normativa ed esplicita,
/// registrata nel pacchetto decisionale e in entrambi i design; la porta
/// [`PublicMessage::OpzioneRifiutata`] e nessun'altra variante.
///
/// # Costruibile in contesto `const`
///
/// Le prevalidazioni di FZ-0.1 e FZ-0.2 dichiarano i propri messaggi come
/// costanti, e i gate lo verificano. Se questo tipo non fosse costruibile in
/// `const`, quelle costanti diventerebbero funzioni e il gate perderebbe la
/// presa.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PublicMessage {
    /// Testo scritto da noi, scelto a compile time.
    Curated(&'static str),
    /// Testo nostro piu' un numero strutturale: «layer 3 fuori dal piano».
    CuratedWith(&'static str, NumeroStrutturale),
    /// Testo nostro, un numero e la sua unita' di misura, entrambi nostri:
    /// «12 colonne oltre il limite 8».
    CuratedBetween(
        &'static str,
        NumeroStrutturale,
        &'static str,
        NumeroStrutturale,
    ),
    /// La ragione tipizzata per cui una capability non c'e'.
    Capability(CapabilityReason),
    /// **L'unica variante che porta testo runtime.**
    ///
    /// Il token e' bounded, scappato e troncato alla costruzione, e si conia
    /// solo dentro `format_options::valida_opzioni`. Vedi l'errata normativa
    /// del 2026-08-19.
    ///
    /// Gli altri campi **non** sono testo runtime: `driver`, `testo` e
    /// `dettaglio` sono `&'static str`, e `ammesse` e' un elenco di
    /// `&'static str` — le chiavi che lo schema dichiara. La lista e' costruita
    /// a runtime, i suoi elementi no, ed e' la differenza che conta: nessuno di
    /// quei caratteri viene dal file o da una dipendenza.
    OpzioneRifiutata {
        driver: &'static str,
        testo: &'static str,
        token: crate::format_options::RejectedOptionToken,
        dettaglio: &'static str,
        ammesse: Vec<&'static str>,
    },
}

impl std::fmt::Display for PublicMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Curated(testo) => f.write_str(testo),
            Self::CuratedWith(testo, numero) => write!(f, "{testo} {numero}"),
            Self::CuratedBetween(testo, primo, mezzo, secondo) => {
                write!(f, "{testo} {primo} {mezzo} {secondo}")
            }
            Self::Capability(reason) => write!(f, "capability non disponibile: {reason:?}"),
            Self::OpzioneRifiutata {
                driver,
                testo,
                token,
                dettaglio,
                ammesse,
            } => {
                write!(f, "{driver}: {testo} '{token}'")?;
                if !dettaglio.is_empty() {
                    write!(f, " {dettaglio}")?;
                }
                // «nessuna» invece del silenzio: un elenco vuoto e' esso
                // stesso l'informazione — questo driver non accetta alcuna
                // opzione in questa fase — e ometterlo lascerebbe credere che
                // l'elenco sia stato dimenticato.
                if ammesse.is_empty() {
                    f.write_str("; accettate: nessuna")
                } else {
                    write!(f, "; accettate: {}", ammesse.join(", "))
                }
            }
        }
    }
}

/// Tetto globale sul messaggio pubblico: **2048 byte UTF-8 del valore
/// decodificato**.
///
/// # Cosa garantisce, e cosa no
///
/// Garantisce che `PlenoraIoError::message` — la `String` Rust, cioe' il
/// **valore decodificato** — non superi 2048 byte UTF-8.
///
/// **Non** garantisce che il campo occupi 2048 byte una volta serializzato in
/// JSON. L'escaping espande: una virgoletta diventa due byte, un carattere di
/// controllo diventa sei (``). Un messaggio al limite fatto di soli
/// controlli si serializza in circa dodici kilobyte piu' le virgolette.
///
/// La prima stesura di questa doc prometteva il limite **sul wire**. Era falso:
/// il codice misura `String::len()`, che e' il valore decodificato, e nessuna
/// misura avveniva dopo la serializzazione. Un tetto che promette una cosa e ne
/// misura un'altra e' peggio di nessun tetto, perche' qualcuno ci dimensiona un
/// buffer.
///
/// Se un giorno servisse anche un limite sul wire, va **dichiarato a parte e
/// misurato dopo la serializzazione**. Oggi non e' promesso, e il test
/// `il_tetto_e_sul_valore_decodificato_non_sul_json` fissa la differenza invece
/// di lasciarla implicita.
///
/// # Perche' byte e non caratteri
///
/// Perche' e' la grandezza che un consumatore puo' usare per dimensionare
/// qualcosa. Contare i caratteri lascerebbe passare un messaggio quattro volte
/// piu' grande a parita' di conteggio.
///
/// # Globale
///
/// Vale su ogni errore, comunque costruito: e' applicato nell'unico punto da
/// cui passano tutti i costruttori, non e' una raccomandazione che ogni sito
/// deve ricordare.
pub const MAX_MESSAGE_BYTES: usize = 2048;

/// Il marcatore che rende visibile un troncamento.
const MARCATORE_TRONCAMENTO: &str = "…";

/// Riporta un messaggio dentro [`MAX_MESSAGE_BYTES`], troncando su un confine
/// di carattere.
///
/// Il troncamento e' deterministico: lo stesso messaggio produce sempre lo
/// stesso taglio. Non tronca a meta' di un carattere UTF-8 — una stringa Rust
/// non lo permetterebbe comunque, ma la ricerca del confine e' esplicita
/// invece che affidata a un `panic` evitato per fortuna.
///
/// Il risultato include il marcatore **dentro** il tetto: la garanzia e' che
/// il valore decodificato non superi mai 2048 byte, non che li superi di tre.
fn limita_messaggio(message: String) -> String {
    if message.len() <= MAX_MESSAGE_BYTES {
        return message;
    }
    let disponibili = MAX_MESSAGE_BYTES - MARCATORE_TRONCAMENTO.len();
    let mut taglio = disponibili;
    while taglio > 0 && !message.is_char_boundary(taglio) {
        taglio -= 1;
    }
    let mut ridotto = String::with_capacity(taglio + MARCATORE_TRONCAMENTO.len());
    ridotto.push_str(&message[..taglio]);
    ridotto.push_str(MARCATORE_TRONCAMENTO);
    ridotto
}

impl PlenoraIoError {
    #[must_use]
    pub fn new(
        category: ErrorCategory,
        phase: ErrorPhase,
        remote_effect: RemoteEffect,
        retry: RetryDisposition,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: IoErrorCode::Generic,
            category,
            phase,
            remote_effect,
            retry,
            driver: None,
            field: None,
            capability_reason: None,
            message: limita_messaggio(message.into()),
            row_diagnostics: None,
        }
    }

    /// Costruttore **redatto**: non accetta testo libero, per costruzione.
    ///
    /// È la via nuova di S9. Coesiste con i costruttori storici finché la
    /// migrazione non arriva a zero in ogni crate; il gate per-crate
    /// (`scripts/check_errori_redatti.py`) impedisce che un crate già migrato
    /// torni indietro.
    ///
    /// Il messaggio è renderizzato **una volta**, alla costruzione, e passa dal
    /// tetto globale come qualunque altro: `plenora-io-error-v1` resta
    /// invariato, perché sul wire finisce la stessa `String` di prima. Ciò che
    /// cambia è chi ha deciso quel testo — un tipo, non un `format!`.
    #[must_use]
    pub fn redatto(
        code: IoErrorCode,
        category: ErrorCategory,
        phase: ErrorPhase,
        remote_effect: RemoteEffect,
        retry: RetryDisposition,
        message: &PublicMessage,
    ) -> Self {
        let mut errore = Self::new(category, phase, remote_effect, retry, message.to_string());
        errore.code = code;
        errore
    }

    /// Un errore WKB redatto.
    #[must_use]
    pub fn wkb_redatto(message: &PublicMessage) -> Self {
        Self::redatto(
            IoErrorCode::Wkb,
            ErrorCategory::DataMapping,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        )
    }

    /// Una quota superata, redatta.
    #[must_use]
    pub fn limite_redatto(message: &PublicMessage) -> Self {
        Self::redatto(
            IoErrorCode::LimitExceeded,
            ErrorCategory::ResourceLimit,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        )
    }

    /// Una violazione di contratto, redatta.
    #[must_use]
    pub fn contratto_redatto(message: &PublicMessage) -> Self {
        Self::redatto(
            IoErrorCode::Contract,
            ErrorCategory::InvalidPlan,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        )
    }

    /// Attacca il contesto strutturato.
    ///
    /// `ErrorContext` è semantico e non sa niente del wire: da qui si
    /// travasano nei campi che `plenora-io-error-v1` già conosce, e nient'altro
    /// esce. Sarà il DTO di contracts-next a decidere il resto.
    #[must_use]
    pub fn con_contesto(mut self, context: &ErrorContext) -> Self {
        if let Some(driver) = context.driver() {
            self.driver = Some(driver.to_owned());
        }
        if let Some(identificatore) = context.identificatore() {
            self.field = Some(identificatore.to_string());
        }
        if let Some(reason) = context.capability_reason() {
            self.capability_reason = Some(reason);
        }
        self
    }

    #[must_use]
    pub fn with_row_diagnostics(mut self, diagnostics: RowDiagnostics) -> Self {
        self.row_diagnostics = Some(Box::new(diagnostics));
        self
    }

    #[must_use]
    pub const fn during(mut self, phase: ErrorPhase) -> Self {
        self.phase = phase;
        self
    }

    #[must_use]
    pub const fn with_effect(mut self, effect: RemoteEffect, retry: RetryDisposition) -> Self {
        self.remote_effect = effect;
        self.retry = retry;
        self
    }

    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        !matches!(self.retry, RetryDisposition::Never)
    }

    #[must_use]
    pub fn capability(
        driver: &'static str,
        field: Option<String>,
        reason: CapabilityReason,
        detail: impl Into<String>,
    ) -> Self {
        let mut error = Self::new(
            ErrorCategory::Unsupported,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            detail,
        );
        error.driver = Some(driver.to_owned());
        error.code = IoErrorCode::Capability;
        error.field = field;
        error.capability_reason = Some(reason);
        error
    }

    #[must_use]
    pub fn format(driver: &'static str, reason: impl Into<String>) -> Self {
        let mut error = Self::new(
            ErrorCategory::DataMapping,
            ErrorPhase::Read,
            RemoteEffect::None,
            RetryDisposition::Never,
            reason,
        );
        error.driver = Some(driver.to_owned());
        error.code = IoErrorCode::Format;
        error
    }

    #[must_use]
    pub fn crs_unresolved(driver: &'static str, raw: &RawCrs) -> Self {
        let mut error = Self::new(
            ErrorCategory::Crs,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            format!(
                "CRS dichiarato ma non risolto (authority_hint_bytes={}, definition_bytes={})",
                raw.authority_hint.as_ref().map_or(0, String::len),
                raw.definition.as_ref().map_or(0, String::len)
            ),
        );
        error.driver = Some(driver.to_owned());
        error.code = IoErrorCode::CrsUnresolved;
        error
    }

    #[must_use]
    pub fn reader_busy(driver: &'static str, layer: u32) -> Self {
        let mut error = Self::new(
            ErrorCategory::Conflict,
            ErrorPhase::Prepare,
            RemoteEffect::None,
            RetryDisposition::Never,
            format!("reader già attivo per il layer {layer}"),
        );
        error.driver = Some(driver.to_owned());
        error.code = IoErrorCode::ReaderBusy;
        error
    }

    #[must_use]
    pub fn projection_unsupported(driver: &'static str) -> Self {
        let mut error = Self::new(
            ErrorCategory::Unsupported,
            ErrorPhase::Prepare,
            RemoteEffect::None,
            RetryDisposition::Never,
            "projection Required non supportata",
        );
        error.driver = Some(driver.to_owned());
        error.code = IoErrorCode::ProjectionUnsupported;
        error
    }

    #[must_use]
    pub fn cancelled(phase: ErrorPhase, deadline: bool) -> Self {
        let mut error = Self::new(
            if deadline {
                ErrorCategory::Timeout
            } else {
                ErrorCategory::Cancelled
            },
            phase,
            RemoteEffect::None,
            RetryDisposition::Never,
            if deadline {
                "operazione interrotta per deadline"
            } else {
                "operazione annullata dal chiamante"
            },
        );
        error.code = IoErrorCode::Cancelled;
        error
    }

    // Costruttori con il nome storico: mantengono sorgenti compatibili i siti
    // semplici, ma producono sempre il nuovo envelope a quattro assi.
    #[allow(non_snake_case)]
    #[must_use]
    pub fn Contract(message: String) -> Self {
        let mut error = Self::new(
            ErrorCategory::InvalidPlan,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        );
        error.code = IoErrorCode::Contract;
        error
    }

    #[allow(non_snake_case)]
    #[must_use]
    pub fn Unsupported(message: String) -> Self {
        let mut error = Self::new(
            ErrorCategory::Unsupported,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        );
        error.code = IoErrorCode::Unsupported;
        error
    }

    #[allow(non_snake_case)]
    #[must_use]
    pub fn Schema(message: String) -> Self {
        let mut error = Self::new(
            ErrorCategory::Schema,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        );
        error.code = IoErrorCode::Schema;
        error
    }

    #[allow(non_snake_case)]
    #[must_use]
    pub fn Crs(message: String) -> Self {
        let mut error = Self::new(
            ErrorCategory::Crs,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        );
        error.code = IoErrorCode::Crs;
        error
    }

    #[allow(non_snake_case)]
    #[must_use]
    pub fn Wkb(message: String) -> Self {
        let mut error = Self::new(
            ErrorCategory::DataMapping,
            ErrorPhase::Read,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        );
        error.code = IoErrorCode::Wkb;
        error
    }

    #[allow(non_snake_case)]
    #[must_use]
    pub fn LimitExceeded(message: String) -> Self {
        let mut error = Self::new(
            ErrorCategory::ResourceLimit,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        );
        error.code = IoErrorCode::LimitExceeded;
        error
    }

    #[allow(non_snake_case)]
    #[must_use]
    pub fn OutputExists(_message: String) -> Self {
        let mut error = Self::new(
            ErrorCategory::Conflict,
            ErrorPhase::Commit,
            RemoteEffect::None,
            RetryDisposition::Never,
            "destinazione già esistente",
        );
        error.code = IoErrorCode::OutputExists;
        error
    }

    #[allow(non_snake_case)]
    #[must_use]
    // Firma per valore: il costruttore consuma l'errore sorgente ed e' parte
    // dell'identita' pubblica del bordo I/O.
    #[allow(clippy::needless_pass_by_value)]
    pub fn Io(error: std::io::Error) -> Self {
        let mut result = Self::new(
            ErrorCategory::Io,
            ErrorPhase::Read,
            RemoteEffect::None,
            RetryDisposition::Never,
            format!("errore filesystem ({:?})", error.kind()),
        );
        result.code = IoErrorCode::Io;
        result
    }

    #[allow(non_snake_case)]
    #[must_use]
    // Firma per valore: il costruttore consuma l'errore sorgente ed e' parte
    // dell'identita' pubblica del bordo I/O.
    #[allow(clippy::needless_pass_by_value)]
    pub fn Json(error: serde_json::Error) -> Self {
        let mut result = Self::new(
            ErrorCategory::DataMapping,
            ErrorPhase::Read,
            RemoteEffect::None,
            RetryDisposition::Never,
            format!(
                "documento JSON non valido alla riga {}, colonna {}",
                error.line(),
                error.column()
            ),
        );
        result.code = IoErrorCode::Json;
        result
    }
}

impl fmt::Display for PlenoraIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} durante {:?} (effetto={:?}, retry={:?}): {}",
            self.category, self.phase, self.remote_effect, self.retry, self.message
        )
    }
}

impl std::error::Error for PlenoraIoError {}

impl From<std::io::Error> for PlenoraIoError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for PlenoraIoError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::{
        RowDiagnosticExample, RowDiagnosticKey, RowDiagnosticKeyState, RowDiagnosticKeyValue,
        RowDiagnosticScope, RowDiagnosticsCompleteness, ROW_DIAGNOSTICS_CONTRACT,
        ROW_DIAGNOSTICS_INDEX_BASIS,
    };

    /// L'identificatore nasce dallo schema, e solo da lì.
    ///
    /// I casi negativi contano quanto quello positivo: un indice fuori
    /// intervallo o un nome non nominabile producono **nessun identificatore**,
    /// non uno inventato o troncato. Un nome troncato somiglia a un nome vero,
    /// ed è il modo in cui un errore indica il campo sbagliato.
    #[test]
    fn l_identificatore_viene_dal_contratto_validato() {
        use arrow_schema::{DataType, Field, Schema};

        let schema = Schema::new(vec![
            Field::new("geometry", DataType::Binary, true),
            Field::new("nome", DataType::Utf8, true),
        ]);

        let primo = ContractIdentifier::from_schema_field(&schema, crate::contract::FieldId(0))
            .expect("il campo 0 esiste");
        assert_eq!(primo.to_string(), "geometry");
        let secondo = ContractIdentifier::from_schema_field(&schema, crate::contract::FieldId(1))
            .expect("il campo 1 esiste");
        assert_eq!(secondo.to_string(), "nome");

        // Indice fuori intervallo: nessun identificatore.
        assert!(
            ContractIdentifier::from_schema_field(&schema, crate::contract::FieldId(2)).is_none()
        );

        // Nome vuoto: non nominabile.
        let vuoto = Schema::new(vec![Field::new("", DataType::Utf8, true)]);
        assert!(
            ContractIdentifier::from_schema_field(&vuoto, crate::contract::FieldId(0)).is_none()
        );

        // Nome oltre il tetto: nessun identificatore, non uno troncato.
        let lunghissimo = Schema::new(vec![Field::new(
            "n".repeat(MAX_IDENTIFICATORE + 1),
            DataType::Utf8,
            true,
        )]);
        assert!(
            ContractIdentifier::from_schema_field(&lunghissimo, crate::contract::FieldId(0))
                .is_none(),
            "meglio non nominare che nominare a meta'"
        );

        // Esattamente al tetto: nominabile.
        let al_limite = Schema::new(vec![Field::new(
            "n".repeat(MAX_IDENTIFICATORE),
            DataType::Utf8,
            true,
        )]);
        assert!(
            ContractIdentifier::from_schema_field(&al_limite, crate::contract::FieldId(0))
                .is_some()
        );
    }

    /// Tripwire sull'indipendenza dal wire — **non** la sua prova principale.
    ///
    /// Le garanzie vere sono tre, e nessuna è questo test: i campi sono privati
    /// con costruttori controllati, il tipo **non implementa `Serialize`**, e la
    /// traduzione verso il wire vive in un DTO separato. Sono proprietà del
    /// tipo e del grafo dei moduli, verificate dal compilatore.
    ///
    /// Questo test guarda il `Debug` e cerca i nomi del contratto di
    /// destinazione. È un allarme a buon mercato per un caso specifico — un
    /// campo rinominato `provider` per comodità del DTO — e vale quanto un
    /// allarme: non dimostra l'indipendenza, segnala un modo particolare di
    /// perderla. Attribuirgli più forza sarebbe descrivere male ciò che
    /// protegge il tipo.
    #[test]
    fn il_contesto_non_conosce_i_nomi_del_wire() {
        let contesto = ErrorContext::nuovo()
            .con_driver("geoparquet")
            .con_layer(crate::contract::LayerId(0))
            .con_campo(crate::contract::FieldId(2))
            .con_capability(CapabilityReason::TypeNotRepresentable);

        let forma = format!("{contesto:?}");
        for del_wire in ["provider", "details", "row_diagnostics", "message"] {
            assert!(
                !forma.contains(del_wire),
                "il contesto semantico non deve nominare '{del_wire}': {forma}"
            );
        }

        assert_eq!(contesto.driver(), Some("geoparquet"));
        assert_eq!(contesto.layer(), Some(crate::contract::LayerId(0)));
        assert_eq!(contesto.campo(), Some(crate::contract::FieldId(2)));
        assert_eq!(
            contesto.capability_reason(),
            Some(CapabilityReason::TypeNotRepresentable)
        );
        assert!(contesto.identificatore().is_none());

        // Vuoto è vuoto: nessun campo si popola da solo.
        let vuoto = ErrorContext::nuovo();
        assert!(vuoto.driver().is_none());
        assert!(vuoto.layer().is_none());
        assert!(vuoto.campo().is_none());
        assert!(vuoto.capability_reason().is_none());
    }

    /// Il tetto di 2048 byte vale su **ogni** errore, comunque costruito.
    ///
    /// Non è una raccomandazione che ogni sito deve ricordare: è applicato
    /// nell'unico punto da cui passano tutti i costruttori, e il test lo
    /// verifica su tutte le forme pubbliche invece che su una.
    #[test]
    fn nessun_errore_supera_il_tetto_del_messaggio() {
        let enorme = "x".repeat(10_000);
        let costruiti = [
            PlenoraIoError::new(
                ErrorCategory::Internal,
                ErrorPhase::Validate,
                RemoteEffect::None,
                RetryDisposition::Never,
                enorme.clone(),
            ),
            PlenoraIoError::Contract(enorme.clone()),
            PlenoraIoError::Unsupported(enorme.clone()),
            PlenoraIoError::Schema(enorme.clone()),
            PlenoraIoError::Crs(enorme.clone()),
            PlenoraIoError::Wkb(enorme.clone()),
            PlenoraIoError::LimitExceeded(enorme.clone()),
            PlenoraIoError::format("prova", enorme.clone()),
            PlenoraIoError::capability(
                "prova",
                None,
                CapabilityReason::TypeNotRepresentable,
                enorme,
            ),
        ];
        for errore in &costruiti {
            assert!(
                errore.message.len() <= MAX_MESSAGE_BYTES,
                "messaggio da {} byte, tetto {MAX_MESSAGE_BYTES}",
                errore.message.len()
            );
            assert!(
                errore.message.ends_with('…'),
                "un messaggio troncato deve dirlo: {}",
                &errore.message[errore.message.len().saturating_sub(16)..]
            );
        }
    }

    /// Il tetto vale sul valore **decodificato**, non sul JSON serializzato.
    ///
    /// È una controprova, non una verifica: misura la differenza fra ciò che il
    /// tetto garantisce e ciò che finisce sul wire, così nessuno la deduca dal
    /// nome della costante.
    ///
    /// L'escaping JSON espande — `"` diventa due byte, un carattere di
    /// controllo ne diventa sei — quindi un messaggio al limite si serializza
    /// più lungo del limite. Il test lo **dimostra** e lo pinna: se un giorno
    /// servisse anche un tetto sul wire, va dichiarato a parte e misurato qui,
    /// dopo la serializzazione. Oggi non è promesso.
    #[test]
    fn il_tetto_e_sul_valore_decodificato_non_sul_json() {
        // Tre input al limite, di espansione crescente.
        let casi = [
            ("ascii", "x".repeat(4096)),
            ("virgolette", "\"".repeat(4096)),
            ("controlli", "\u{1}".repeat(4096)),
        ];
        for (nome, grezzo) in casi {
            let errore = PlenoraIoError::Contract(grezzo);

            // Cio' che il tetto garantisce: il valore decodificato.
            assert!(
                errore.message.len() <= MAX_MESSAGE_BYTES,
                "{nome}: valore decodificato da {} byte",
                errore.message.len()
            );

            // Cio' che finisce sul wire, misurato dopo la serializzazione.
            let serializzato =
                serde_json::to_string(&errore.message).expect("il messaggio si serializza");
            // `to_string` di una String include le virgolette: le tolgo per
            // misurare il solo valore, che e' cio' di cui si sta parlando.
            let sul_wire = serializzato.len() - 2;

            match nome {
                // L'ASCII non si espande: qui i due numeri coincidono, ed e'
                // il caso che rende invisibile la differenza se lo si guarda
                // da solo.
                "ascii" => assert_eq!(sul_wire, errore.message.len()),
                // Le virgolette raddoppiano.
                "virgolette" => assert!(
                    sul_wire > errore.message.len(),
                    "le virgolette devono espandersi: {sul_wire} vs {}",
                    errore.message.len()
                ),
                // I controlli sestuplicano: e' il caso peggiore, ed e' quello
                // che smentisce la promessa sbagliata.
                "controlli" => assert!(
                    sul_wire > MAX_MESSAGE_BYTES * 5,
                    "i controlli devono espandersi molto: {sul_wire} byte sul wire \
                     contro {} decodificati",
                    errore.message.len()
                ),
                _ => unreachable!(),
            }
        }
    }

    /// Il troncamento è deterministico e non spezza un carattere.
    ///
    /// Multibyte perché è lì che un taglio a byte fisso romperebbe: 2045 non
    /// cade su un confine di `à`, e la ricerca del confine deve tornare
    /// indietro invece di panicare.
    #[test]
    fn il_troncamento_e_deterministico_e_rispetta_i_caratteri() {
        let multibyte = "à".repeat(10_000);
        let primo = PlenoraIoError::Contract(multibyte.clone()).message;
        let secondo = PlenoraIoError::Contract(multibyte).message;
        assert_eq!(primo, secondo, "il troncamento deve essere deterministico");
        assert!(primo.len() <= MAX_MESSAGE_BYTES);
        // Se il taglio avesse spezzato un carattere, la stringa non esisterebbe
        // nemmeno: qui si verifica che il contenuto sia quello atteso.
        assert!(primo.starts_with("àà"));
        assert!(primo.ends_with('…'));

        // Un messaggio sotto il tetto non viene toccato né marcato.
        let corto = PlenoraIoError::Contract("piano non valido".to_owned()).message;
        assert_eq!(corto, "piano non valido");
    }

    #[test]
    fn error_serializes_complete_bounded_read_diagnostics() {
        let diagnostics = RowDiagnostics {
            contract: ROW_DIAGNOSTICS_CONTRACT.to_owned(),
            scope: RowDiagnosticScope::Read,
            index_basis: ROW_DIAGNOSTICS_INDEX_BASIS.to_owned(),
            completeness: RowDiagnosticsCompleteness::Complete,
            observed_total: 3,
            total: Some(3),
            counts: BTreeMap::from([
                ("shapefile.inner_ring_without_outer".to_owned(), 2),
                ("shapefile.unclosed_ring".to_owned(), 1),
            ]),
            examples_limit: 2,
            examples_truncated: true,
            examples: vec![
                RowDiagnosticExample {
                    source_index: 17,
                    cause: "shapefile.inner_ring_without_outer".to_owned(),
                    column: None,
                    key: Some(RowDiagnosticKey {
                        field: "ID_PART".to_owned(),
                        state: RowDiagnosticKeyState::Value,
                        value: Some(RowDiagnosticKeyValue::String("9007199254741009".to_owned())),
                    }),
                    write_state: None,
                },
                RowDiagnosticExample {
                    source_index: 89,
                    cause: "shapefile.unclosed_ring".to_owned(),
                    column: None,
                    key: None,
                    write_state: None,
                },
            ],
            knowledge_limits: None,
            input_total: None,
            diagnostic_state_counts: None,
            write_outcome: None,
        };
        let error = PlenoraIoError::format("shp", "3 righe Shapefile non valide")
            .with_row_diagnostics(diagnostics);

        let value = serde_json::to_value(error).unwrap();
        assert_eq!(
            value["row_diagnostics"]["contract"],
            ROW_DIAGNOSTICS_CONTRACT
        );
        assert_eq!(value["row_diagnostics"]["observed_total"], 3);
        assert_eq!(
            value["row_diagnostics"]["examples"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            value["row_diagnostics"]["examples"][0]["key"]["value"],
            "9007199254741009"
        );
        assert!(value["row_diagnostics"]["input_total"].is_null());
    }

    #[test]
    fn unresolved_authority_error_does_not_require_or_expose_a_definition() {
        let raw = RawCrs::from_authority_hint("EPSG:99999".to_owned());
        let error = PlenoraIoError::crs_unresolved("ipc", &raw);

        assert!(error.message.contains("definition_bytes=0"));
        assert!(!error.message.contains("EPSG:99999"));
    }

    #[test]
    fn timeout_after_commit_keeps_cause_effect_and_recovery_separate() {
        let error = PlenoraIoError::new(
            ErrorCategory::Timeout,
            ErrorPhase::Commit,
            RemoteEffect::Unknown,
            RetryDisposition::RequiresRecovery,
            "esito commit non verificabile",
        );
        assert_eq!(error.category, ErrorCategory::Timeout);
        assert_eq!(error.remote_effect, RemoteEffect::Unknown);
        assert_eq!(error.retry, RetryDisposition::RequiresRecovery);
        assert!(error.is_retryable());
    }

    #[test]
    fn filesystem_messages_do_not_expose_paths() {
        let source = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "C:\\secret\\customer.geojson",
        );
        let error = PlenoraIoError::from(source);
        assert!(!error.to_string().contains("customer.geojson"));
        assert_eq!(error.category, ErrorCategory::Io);
    }

    #[test]
    fn four_axis_error_roundtrips_and_rejects_unknown_fields() {
        let error = PlenoraIoError::new(
            ErrorCategory::Timeout,
            ErrorPhase::Commit,
            RemoteEffect::Unknown,
            RetryDisposition::RequiresRecovery,
            "esito commit non verificabile",
        );
        let value = serde_json::to_value(&error).unwrap();
        let decoded: PlenoraIoError = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decoded, error);

        let mut object = value.as_object().unwrap().clone();
        object.insert("future_axis".to_owned(), serde_json::Value::Null);
        assert!(
            serde_json::from_value::<PlenoraIoError>(serde_json::Value::Object(object)).is_err()
        );
    }

    #[test]
    fn retry_disposition_uses_the_shared_tagged_object_shape() {
        let cases = [
            (
                RetryDisposition::Never,
                serde_json::json!({"kind": "never"}),
            ),
            (RetryDisposition::Safe, serde_json::json!({"kind": "safe"})),
            (
                RetryDisposition::RequiresIdempotencyKey,
                serde_json::json!({"kind": "requires_idempotency_key"}),
            ),
            (
                RetryDisposition::RequiresRecovery,
                serde_json::json!({"kind": "requires_recovery"}),
            ),
            (
                RetryDisposition::After(2_750),
                serde_json::json!({"kind": "after", "delay_ms": 2_750}),
            ),
        ];

        for (retry, expected) in cases {
            assert_eq!(serde_json::to_value(retry).unwrap(), expected);
        }
    }
}
