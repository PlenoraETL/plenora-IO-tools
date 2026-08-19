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

/// Tetto globale sulla lunghezza del messaggio pubblico, in **byte**.
///
/// Byte e non caratteri perche' il vincolo e' sul wire: `plenora-io-error-v1`
/// e' JSON UTF-8, e cio' che un consumatore deve poter dimensionare e' il
/// buffer, non il conteggio dei grafemi. Un messaggio di 2048 caratteri
/// multibyte occuperebbe otto kilobyte, e il tetto avrebbe promesso una cosa
/// misurandone un'altra.
///
/// Il tetto e' **globale e assoluto**: vale su ogni errore, comunque
/// costruito, perche' e' applicato nell'unico punto da cui passano tutti i
/// costruttori. Non e' una raccomandazione che ogni sito deve ricordare.
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
/// il campo `message` del wire non superi mai 2048 byte, non che li superi di
/// tre.
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
