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
///
/// # La via legacy non esiste piu', e il compilatore lo impone
///
/// Fino alla chiusura di S9 questo tipo aveva costruttori che accettavano
/// `String` e `impl Into<String>`. Sono stati rimossi: **la garanzia non e' un
/// gate ne' una convenzione, e' l'assenza della funzione**.
///
/// I blocchi che seguono compilano come **crate separati** che dipendono da
/// `plenora-io-model`: vedono esattamente cio' che vede un consumatore
/// esterno, e nient'altro.
///
/// ## Come sono costruite queste prove, e perche' cosi'
///
/// **Un `compile_fail` da solo prova poco**, e va detto invece che lasciato
/// intendere: passa se il blocco non compila per una ragione *qualunque* — un
/// import sbagliato, un nome di tipo storpiato, un argomento in piu'.
/// Annotare il codice d'errore atteso non aiuta: e' stato verificato che
/// `compile_fail,E0277` resta **verde** dove l'errore vero e' `E0624`, quindi
/// rustdoc non lo impone.
///
/// Le prove sono percio' in **coppie**. Ogni blocco `compile_fail` e' il
/// blocco che lo precede — che compila e passa le sue asserzioni — **piu' una
/// riga**: quella che usa l'API vietata. Stessi import, stessi tipi, stesse
/// chiamate permesse. Se il blocco negativo fallisse per una ragione diversa
/// dalla riga aggiunta, il positivo fallirebbe con lui, e la coppia
/// diventerebbe rossa.
///
/// La non vacuita' e' quindi **strutturale**, non affermata. La seconda prova,
/// indipendente da rustdoc, e' il gate `scripts/check_errori_redatti.py`, che
/// verifica nel sorgente che le definizioni non siano tornate a esistere.
///
/// ## Che cosa questa garanzia **non** e'
///
/// `&'static str` garantisce la **durata**, non la **provenienza**. Un
/// chiamante deliberato puo' promuovere testo runtime a `'static` con
/// `Box::leak` e infilarlo in un `PublicMessage::Curated` senza che il
/// compilatore obietti:
///
/// ```
/// // DIMOSTRAZIONE-LIMITE-STATIC — l'unica occorrenza autorizzata di
/// // `Box::leak` nel repository. `scripts/check_niente_leak.py` la attesta
/// // per questo marcatore, non per numero di riga, e diventa rosso sia se ne
/// // compare un'altra sia se questa sparisce.
/// use plenora_io_model::{PlenoraIoError, PublicMessage};
/// let runtime = format!("valore {} dal payload", 42);
/// let promosso: &'static str = Box::leak(runtime.into_boxed_str());
/// let errore = PlenoraIoError::contratto_redatto(&PublicMessage::Curated(promosso));
/// assert!(errore.message.contains("dal payload"));
/// ```
///
/// **Quel doctest compila, e passa.** E' qui apposta: una garanzia descritta
/// piu' forte di com'e' e' peggio di una garanzia dichiarata con il suo limite.
///
/// La promessa realistica di S9 e' percio':
///
/// > impedire la propagazione **accidentale** di testo runtime nel workspace,
/// > non rendere crittograficamente inconiabile un messaggio dinamico da
/// > codice ostile.
///
/// I crate sono interni e `publish = false`: l'avversario di questo invariante
/// e' la distrazione, non un aggressore. La distrazione e' coperta dal tipo
/// (una `String` non entra da sola) e dal gate `scripts/check_niente_leak.py`,
/// che vieta la promozione in tutto il workspace, **test compresi** — perche'
/// l'unica occorrenza mai esistita era in un test.
///
/// ### Una `String` non entra, nemmeno per il costruttore con il nome storico
///
/// ```
/// use plenora_io_model::{PlenoraIoError, PublicMessage};
/// let permesso = PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
///     "piano non valido",
/// ));
/// assert_eq!(permesso.message, "piano non valido");
/// ```
///
/// ```compile_fail
/// use plenora_io_model::{PlenoraIoError, PublicMessage};
/// let permesso = PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
///     "piano non valido",
/// ));
/// assert_eq!(permesso.message, "piano non valido");
/// let _vietato = PlenoraIoError::Contract(String::from("piano non valido"));
/// ```
///
/// ### Nemmeno passata alla via nuova, dove il tipo la rifiuta
///
/// ```
/// use plenora_io_model::{
///     ErrorCategory, ErrorPhase, IoErrorCode, PlenoraIoError, PublicMessage,
///     RemoteEffect, RetryDisposition,
/// };
/// let permesso = PlenoraIoError::redatto(
///     IoErrorCode::Generic,
///     ErrorCategory::InvalidPlan,
///     ErrorPhase::Validate,
///     RemoteEffect::None,
///     RetryDisposition::Never,
///     &PublicMessage::Curated("piano non valido"),
/// );
/// assert_eq!(permesso.category, ErrorCategory::InvalidPlan);
/// ```
///
/// ```compile_fail
/// use plenora_io_model::{
///     ErrorCategory, ErrorPhase, IoErrorCode, PlenoraIoError, PublicMessage,
///     RemoteEffect, RetryDisposition,
/// };
/// let permesso = PlenoraIoError::redatto(
///     IoErrorCode::Generic,
///     ErrorCategory::InvalidPlan,
///     ErrorPhase::Validate,
///     RemoteEffect::None,
///     RetryDisposition::Never,
///     &PublicMessage::Curated("piano non valido"),
/// );
/// assert_eq!(permesso.category, ErrorCategory::InvalidPlan);
/// let testo = String::from("piano non valido");
/// let _vietato = PlenoraIoError::redatto(
///     IoErrorCode::Generic,
///     ErrorCategory::InvalidPlan,
///     ErrorPhase::Validate,
///     RemoteEffect::None,
///     RetryDisposition::Never,
///     &testo,
/// );
/// ```
///
/// ### Un `&format!(…)` non entra: e' la scorciatoia piu' probabile, perche' somiglia a un `&str` ed e' cio' che ogni sito migrato faceva prima
///
/// ```
/// use plenora_io_model::{PlenoraIoError, PublicMessage};
/// let permesso = PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
///     "piano non valido",
/// ));
/// assert_eq!(permesso.message, "piano non valido");
/// ```
///
/// ```compile_fail
/// use plenora_io_model::{PlenoraIoError, PublicMessage};
/// let permesso = PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
///     "piano non valido",
/// ));
/// assert_eq!(permesso.message, "piano non valido");
/// let indice = 3_u64;
/// let _vietato = PlenoraIoError::schema_redatto(&format!("layer {indice} non valido"));
/// ```
///
/// ### `format`, che accettava `impl Into<String>`, non esiste piu'
///
/// ```
/// use plenora_io_model::{PlenoraIoError, PublicMessage};
/// let permesso = PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
///     "piano non valido",
/// ));
/// assert_eq!(permesso.message, "piano non valido");
/// ```
///
/// ```compile_fail
/// use plenora_io_model::{PlenoraIoError, PublicMessage};
/// let permesso = PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
///     "piano non valido",
/// ));
/// assert_eq!(permesso.message, "piano non valido");
/// let _vietato = PlenoraIoError::format("shp", "riga non valida");
/// ```
///
/// ### `new` non e' piu' raggiungibile: era la base di tutti e imponeva `code = Generic`
///
/// ```
/// use plenora_io_model::{
///     ErrorCategory, ErrorPhase, IoErrorCode, PlenoraIoError, PublicMessage,
///     RemoteEffect, RetryDisposition,
/// };
/// let permesso = PlenoraIoError::redatto(
///     IoErrorCode::Generic,
///     ErrorCategory::InvalidPlan,
///     ErrorPhase::Validate,
///     RemoteEffect::None,
///     RetryDisposition::Never,
///     &PublicMessage::Curated("piano non valido"),
/// );
/// assert_eq!(permesso.category, ErrorCategory::InvalidPlan);
/// ```
///
/// ```compile_fail
/// use plenora_io_model::{
///     ErrorCategory, ErrorPhase, IoErrorCode, PlenoraIoError, PublicMessage,
///     RemoteEffect, RetryDisposition,
/// };
/// let permesso = PlenoraIoError::redatto(
///     IoErrorCode::Generic,
///     ErrorCategory::InvalidPlan,
///     ErrorPhase::Validate,
///     RemoteEffect::None,
///     RetryDisposition::Never,
///     &PublicMessage::Curated("piano non valido"),
/// );
/// assert_eq!(permesso.category, ErrorCategory::InvalidPlan);
/// let _vietato = PlenoraIoError::new(
///     ErrorCategory::Internal,
///     ErrorPhase::Validate,
///     RemoteEffect::None,
///     RetryDisposition::Never,
///     "qualcosa",
/// );
/// ```
///
/// ### E `LimitExceeded`, `Unsupported`, `Schema`, `Crs`, `Wkb`, `OutputExists`, `capability`, `crs_unresolved` — uno per tutti, perche' un doctest per ciascuno proverebbe la stessa cosa nove volte
///
/// ```
/// use plenora_io_model::{PlenoraIoError, PublicMessage};
/// let permesso = PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
///     "piano non valido",
/// ));
/// assert_eq!(permesso.message, "piano non valido");
/// ```
///
/// ```compile_fail
/// use plenora_io_model::{PlenoraIoError, PublicMessage};
/// let permesso = PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
///     "piano non valido",
/// ));
/// assert_eq!(permesso.message, "piano non valido");
/// let _vietato = PlenoraIoError::LimitExceeded(String::from("oltre il tetto"));
/// ```
///
/// ## La controprova positiva, sulla superficie intera
///
/// I preamboli sopra provano che la via nuova compila; questo blocco prova che
/// **serve a qualcosa**: i valori tipizzati arrivano nel messaggio e i quattro
/// assi restano scegliibili uno per uno.
///
/// ```
/// use plenora_io_model::{
///     ErrorCategory, ErrorPhase, IoErrorCode, NumeroStrutturale, PlenoraIoError,
///     PublicMessage, RemoteEffect, RetryDisposition,
/// };
///
/// // Testo curato, scelto a compile time.
/// let semplice = PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
///     "piano non valido",
/// ));
/// assert_eq!(semplice.code, IoErrorCode::Contract);
/// assert_eq!(semplice.message, "piano non valido");
///
/// // Un numero strutturale entra: e' un numero, non testo.
/// let con_indice = PlenoraIoError::schema_redatto(&PublicMessage::CuratedWith(
///     "layer non valido all'indice",
///     NumeroStrutturale::Indice(3),
/// ));
/// assert_eq!(con_indice.message, "layer non valido all'indice 3");
///
/// // E i quattro assi restano scegliibili uno per uno.
/// let esplicito = PlenoraIoError::redatto(
///     IoErrorCode::Generic,
///     ErrorCategory::Timeout,
///     ErrorPhase::Commit,
///     RemoteEffect::Unknown,
///     RetryDisposition::RequiresRecovery,
///     &PublicMessage::Curated("esito commit non verificabile"),
/// );
/// assert_eq!(esplicito.category, ErrorCategory::Timeout);
/// assert_eq!(esplicito.remote_effect, RemoteEffect::Unknown);
/// assert!(esplicito.is_retryable());
/// ```
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

    /// L'identificatore di una colonna geometrica, preso dal contratto che la
    /// dichiara.
    ///
    /// Passa dal contratto come [`Self::from_layer`], e non dal nome nudo: il
    /// punto del tipo e' che «viene da un contratto validato» sia una
    /// proprieta' della costruzione, non una promessa del chiamante.
    #[must_use]
    pub fn from_geometry_column(
        geometry: &crate::contract::GeometryColumnContract,
    ) -> Option<Self> {
        Self::da_nome_validato(&geometry.name)
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
    /// Due testi nostri, entrambi scelti a compile time: «CRS richiesto
    /// epsg:4326».
    ///
    /// Vale la stessa prova di `Curated`, su entrambi gli argomenti. Una
    /// `String` non entra da nessuno dei due lati:
    ///
    /// ```compile_fail
    /// use plenora_io_model::PublicMessage;
    /// let a_runtime = String::from("valore letto dal file");
    /// let _ = PublicMessage::CuratedPair("prefisso", a_runtime);
    /// ```
    ///
    /// ```compile_fail
    /// use plenora_io_model::PublicMessage;
    /// let a_runtime = String::from("valore letto dal file");
    /// let _ = PublicMessage::CuratedPair(a_runtime, "suffisso");
    /// ```
    ///
    /// Nemmeno un prestito piu' corto di `'static`, che e' il modo in cui il
    /// payload proverebbe a entrare:
    ///
    /// ```compile_fail
    /// use plenora_io_model::PublicMessage;
    /// fn da_runtime(valore: &str) -> PublicMessage {
    ///     PublicMessage::CuratedPair("prefisso", valore)
    /// }
    /// ```
    ///
    /// Nemmeno un `format!`:
    ///
    /// ```compile_fail
    /// use plenora_io_model::PublicMessage;
    /// let _ = PublicMessage::CuratedPair("prefisso", &format!("{}", 1));
    /// ```
    ///
    /// Due letterali si', e in contesto `const`:
    ///
    /// ```
    /// use plenora_io_model::PublicMessage;
    /// const MESSAGGIO: PublicMessage =
    ///     PublicMessage::CuratedPair("encoding non supportato:", "ewkb");
    /// assert_eq!(MESSAGGIO.to_string(), "encoding non supportato: ewkb");
    /// ```
    ///
    /// Non e' un'eccezione alla proprieta': un `&'static str` non puo' venire
    /// dal payload ne' da una dipendenza, e quale dei due statici usare lo
    /// decide comunque il nostro codice. Serve dove il messaggio deve unire
    /// una frase e un codice strutturale — il nome di un enum nostro, la causa
    /// di un rifiuto — che `Curated` da solo non sa mettere insieme.
    CuratedPair(&'static str, &'static str),
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
            Self::CuratedPair(primo, secondo) => write!(f, "{primo} {secondo}"),
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
    /// Base **interna**. Non e' pubblica: dopo S9 nessun consumatore puo'
    /// costruire un errore da testo libero, e questa e' la funzione che lo
    /// permetterebbe. La usano solo i costruttori che compongono il proprio
    /// messaggio da valori tipizzati (`reader_busy`, `cancelled`, `Io`,
    /// `Json`, …), mai da un argomento del chiamante.
    #[must_use]
    fn new(
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

    /// Una capacità non disponibile, redatta.
    ///
    /// Rispecchia `Unsupported` legacy: stessa categoria, stessa fase, stesso
    /// codice. Cambia solo chi ha scelto il testo.
    #[must_use]
    pub fn non_supportato_redatto(message: &PublicMessage) -> Self {
        Self::redatto(
            IoErrorCode::Unsupported,
            ErrorCategory::Unsupported,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        )
    }

    /// Uno schema non conforme, redatto.
    #[must_use]
    pub fn schema_redatto(message: &PublicMessage) -> Self {
        Self::redatto(
            IoErrorCode::Schema,
            ErrorCategory::Schema,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        )
    }

    /// Un CRS non conforme, redatto.
    #[must_use]
    pub fn crs_redatto(message: &PublicMessage) -> Self {
        Self::redatto(
            IoErrorCode::Crs,
            ErrorCategory::Crs,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        )
    }

    /// Un difetto di formato attribuito a un driver, redatto.
    ///
    /// Il driver è `&'static str` per costruzione: viene dal descrittore, non
    /// dal payload.
    #[must_use]
    pub fn formato_redatto(driver: &'static str, message: &PublicMessage) -> Self {
        let mut errore = Self::redatto(
            IoErrorCode::Format,
            ErrorCategory::DataMapping,
            ErrorPhase::Read,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        );
        errore.driver = Some(driver.to_owned());
        errore
    }

    /// Una capacità dichiarata assente dal driver, redatta.
    ///
    /// Il campo, quando c'è, è un [`ContractIdentifier`]: nasce da un contratto
    /// validato, non da una `String` qualunque.
    #[must_use]
    pub fn capability_redatta(
        driver: &'static str,
        field: Option<&ContractIdentifier>,
        reason: CapabilityReason,
        message: &PublicMessage,
    ) -> Self {
        let mut errore = Self::redatto(
            IoErrorCode::Capability,
            ErrorCategory::Unsupported,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        );
        errore.driver = Some(driver.to_owned());
        errore.field = field.map(ToString::to_string);
        errore.capability_reason = Some(reason);
        errore
    }

    /// Un CRS dichiarato ma non risolto, redatto.
    ///
    /// Del `RawCrs` escono **due conteggi di byte**, non il suo contenuto: la
    /// definizione e l'hint di authority vengono dal file, e dire quanto sono
    /// lunghi e' l'informazione che il chiamante non ha senza dire quella che
    /// ha gia'.
    #[must_use]
    pub fn crs_non_risolto_redatto(driver: &'static str, raw: &RawCrs) -> Self {
        let mut errore = Self::redatto(
            IoErrorCode::CrsUnresolved,
            ErrorCategory::Crs,
            ErrorPhase::Validate,
            RemoteEffect::None,
            RetryDisposition::Never,
            &PublicMessage::CuratedBetween(
                "CRS dichiarato ma non risolto: authority_hint di",
                NumeroStrutturale::Conteggio(
                    raw.authority_hint.as_ref().map_or(0, String::len) as u64
                ),
                "byte, definizione di",
                NumeroStrutturale::Conteggio(raw.definition.as_ref().map_or(0, String::len) as u64),
            ),
        );
        errore.driver = Some(driver.to_owned());
        errore
    }

    /// La destinazione esiste già.
    ///
    /// Non prende messaggio: il costruttore storico ignorava il proprio
    /// argomento e ne scriveva uno curato. Qui l'argomento inutile sparisce
    /// invece di restare a suggerire un'influenza che non ha mai avuto.
    #[must_use]
    pub fn destinazione_esistente() -> Self {
        Self::redatto(
            IoErrorCode::OutputExists,
            ErrorCategory::Conflict,
            ErrorPhase::Commit,
            RemoteEffect::None,
            RetryDisposition::Never,
            &PublicMessage::Curated("destinazione già esistente"),
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
mod tests;
