//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

#[test]
fn i_limiti_wkb_predefiniti_sono_quelli_storici() {
    let wkb = WkbLimits::default();
    assert_eq!(wkb.max_cell_bytes, 64 * 1024 * 1024);
    assert_eq!(wkb.max_components, 100_000);
    assert_eq!(wkb.max_depth, 64);
}
