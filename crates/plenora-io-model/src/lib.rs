//! Modello locale di plenora-IO-tools: Arrow, convenzione geometria
//! GeoArrow-WKB, CRS, codec/validazione WKB, limiti, errori e `DataContract`.
//! Non è il futuro crate trasversale `plenora-contracts`.
#![forbid(unsafe_code)]

pub use arrow_array;
pub use arrow_schema;

pub mod cancellation;
pub mod contract;
pub mod crs;
pub mod diagnostics;
pub mod error;
pub mod geometry;
pub mod limits;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod resource;
pub mod wkb;
mod wkb_lossless;

pub use cancellation::{CancellationReason, CancellationToken};
pub use diagnostics::{
    KnownOrUnknownCount, RowDiagnosticColumn, RowDiagnosticExample, RowDiagnosticKey,
    RowDiagnosticKeyState, RowDiagnosticKeyValue, RowDiagnosticScope, RowDiagnosticWriteOutcome,
    RowDiagnosticWriteState, RowDiagnostics, RowDiagnosticsCompleteness,
    WriteDiagnosticStateCounts, ROW_DIAGNOSTICS_CONTRACT, ROW_DIAGNOSTICS_INDEX_BASIS,
    ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE,
};
pub use error::{
    CapabilityReason, ErrorCategory, ErrorPhase, IoErrorCode, PlenoraIoError, RemoteEffect, Result,
    RetryDisposition,
};
pub use resource::{ResourceBudget, ResourceKind, ResourceLease, ResourceLimits};
