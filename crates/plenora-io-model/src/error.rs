use std::fmt;

use serde::Serialize;

use crate::crs::RawCrs;

pub type Result<T> = std::result::Result<T, PlenoraIoError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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
    ReprojectionRequired,
    Nullability,
}

/// Errore specifico del componente IO. Mai valori di cella: soltanto contesto
/// operativo (driver, motivo), non contenuti.
#[derive(Debug)]
pub enum PlenoraIoError {
    Contract(String),
    Unsupported(String),
    Capability {
        driver: &'static str,
        field: Option<String>,
        reason: CapabilityReason,
        detail: String,
    },
    Schema(String),
    Format {
        driver: &'static str,
        reason: String,
    },
    Crs(String),
    CrsUnresolved {
        driver: &'static str,
        raw: RawCrs,
    },
    Wkb(String),
    LimitExceeded(String),
    ReaderBusy {
        driver: &'static str,
        layer: u32,
    },
    ProjectionUnsupported {
        driver: &'static str,
    },
    OutputExists(String),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for PlenoraIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(m) => write!(f, "contratto: {m}"),
            Self::Unsupported(m) => write!(f, "non supportato: {m}"),
            Self::Capability {
                driver,
                field,
                reason,
                detail,
            } => {
                write!(f, "capability {driver}")?;
                if let Some(field) = field {
                    write!(f, " campo '{field}'")?;
                }
                write!(f, " ({reason:?}): {detail}")
            }
            Self::Schema(m) => write!(f, "schema: {m}"),
            Self::Format { driver, reason } => write!(f, "formato {driver}: {reason}"),
            Self::Crs(m) => write!(f, "crs: {m}"),
            Self::CrsUnresolved { driver, raw } => {
                write!(
                    f,
                    "crs non risolto: driver {driver}, authority_hint_bytes={}, definition_bytes={}",
                    raw.authority_hint.as_ref().map_or(0, String::len),
                    raw.definition.len()
                )
            }
            Self::Wkb(m) => write!(f, "wkb: {m}"),
            Self::LimitExceeded(m) => write!(f, "limite superato: {m}"),
            Self::ReaderBusy { driver, layer } => {
                write!(f, "reader già attivo: driver {driver}, layer {layer}")
            }
            Self::ProjectionUnsupported { driver } => {
                write!(f, "projection Required non supportata dal driver {driver}")
            }
            Self::OutputExists(m) => write!(f, "output esistente: {m}"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Json(e) => write!(f, "json: {e}"),
        }
    }
}

impl std::error::Error for PlenoraIoError {}

impl From<std::io::Error> for PlenoraIoError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for PlenoraIoError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
