//! plenora-core — fondamenta condivise fra plenora-IO-tools e plenora-data-tools.
//! Fonte normativa del bordo: Arrow, convenzione geometria GeoArrow-WKB, CRS,
//! codec/validazione WKB, Limits, errori, DataContract.
#![forbid(unsafe_code)]

pub use arrow_array;
pub use arrow_schema;

pub mod contract;
pub mod crs;
pub mod error;
pub mod geometry;
pub mod limits;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod wkb;
mod wkb_lossless;

pub use error::{CapabilityReason, PlenoraError, Result};
