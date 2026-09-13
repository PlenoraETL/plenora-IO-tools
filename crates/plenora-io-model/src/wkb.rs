//! Codec WKB condiviso.
//!
//! L'AST lossless è l'unica implementazione binaria: conserva Z/M/SRID e
//! applica limiti e validazione strutturale. Le funzioni `geo-types` restano
//! adattatori XY compatibili con l'API v1, senza un secondo parser/encoder.

use geo_types::Geometry;

use crate::error::Result;
use crate::limits::WkbLimits;

pub use crate::wkb_lossless::{
    decode_wkb, encode_wkb, encode_wkb_into, encode_wkb_into_bounded, inspect_wkb, membro_ammesso,
    WkbCoordinate, WkbFlavor, WkbGeometry, WkbInspection, WkbValue,
};

/// Serializza una geometria `geo-types` in WKB XY little-endian, riusando il
/// buffer fornito (che viene svuotato prima della scrittura).
///
/// # Errors
///
/// Restituisce [`crate::PlenoraIoError::Wkb`] se la geometria `geo-types` non
/// è rappresentabile nell'AST WKB o se la codifica fallisce.
pub fn to_wkb_into(geometry: &Geometry<f64>, output: &mut Vec<u8>) -> Result<()> {
    let geometry = WkbGeometry::from_geo_xy(geometry)?;
    encode_wkb_into(&geometry, WkbFlavor::Iso, output)
}

/// Serializza una geometria `geo-types` in WKB XY little-endian.
///
/// # Errors
///
/// Gli stessi di [`to_wkb_into`].
pub fn to_wkb(geometry: &Geometry<f64>) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    to_wkb_into(geometry, &mut output)?;
    Ok(output)
}

/// Decodifica WKB ISO/EWKB e proietta intenzionalmente il risultato su XY.
///
/// Per conservare Z, M e SRID usare [`decode_wkb`]. Anche l'adattatore v1 usa
/// il parser autoritativo e quindi rifiuta byte residui e strutture incoerenti.
///
/// # Errors
///
/// Restituisce [`crate::PlenoraIoError::Wkb`] se i byte non sono WKB/EWKB
/// validi, se superano i limiti indicati o se il tipo decodificato non è
/// proiettabile su `geo-types` XY (tipi estesi, Z/M non linearizzabili).
pub fn from_wkb(bytes: &[u8], limits: &WkbLimits) -> Result<Geometry<f64>> {
    decode_wkb(bytes, limits)?.to_geo_xy()
}

#[cfg(test)]
mod tests;
