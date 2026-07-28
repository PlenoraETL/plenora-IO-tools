//! Modello locale di plenora-IO-tools: Arrow, convenzione geometria
//! GeoArrow-WKB, CRS, codec/validazione WKB, limiti, errori e `DataContract`.
//! Non è il futuro crate trasversale `plenora-contracts`.
#![forbid(unsafe_code)]

pub use arrow_array;
pub use arrow_schema;

pub mod cancellation;
pub mod contract;
pub mod crs;
pub mod error;
pub mod geometry;
pub mod limits;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod wkb;
mod wkb_lossless;

pub use cancellation::{CancellationReason, CancellationToken};
pub use error::{
    CapabilityReason, ErrorCategory, ErrorPhase, IoErrorCode, PlenoraIoError, RemoteEffect, Result,
    RetryDisposition,
};
