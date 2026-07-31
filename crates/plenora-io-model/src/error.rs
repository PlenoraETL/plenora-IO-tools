use std::fmt;

use serde::{Deserialize, Serialize};

use crate::crs::RawCrs;

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
    InvalidConfiguration,
    Schema,
    DataMapping,
    Crs,
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
            message: message.into(),
        }
    }

    #[must_use]
    pub fn during(mut self, phase: ErrorPhase) -> Self {
        self.phase = phase;
        self
    }

    #[must_use]
    pub fn with_effect(mut self, effect: RemoteEffect, retry: RetryDisposition) -> Self {
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
