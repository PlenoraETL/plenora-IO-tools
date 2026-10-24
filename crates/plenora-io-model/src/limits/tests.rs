//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

/// Byte e componenti sono quelli storici; la profondita' non piu' (era 64): e'
/// quella di GEO-011, e il default non la supera.
#[test]
fn i_limiti_wkb_predefiniti_sono_quelli_storici_e_del_contratto() {
    let wkb = WkbLimits::default();
    assert_eq!(wkb.max_cell_bytes, 64 * 1024 * 1024);
    assert_eq!(wkb.max_components, 100_000);
    assert_eq!(wkb.max_depth, PROFONDITA_WKB_CONTRATTO);
    assert_eq!(PROFONDITA_WKB_CONTRATTO, 32);
}
