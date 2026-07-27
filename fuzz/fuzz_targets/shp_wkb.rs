#![no_main]
//! Coverage-guided sulla conversione WKB XY/XYZ/XYM/XYZM ⇄ shape ESRI.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Err = WKB o topologia non rappresentabile; un panic è un finding.
    let _ = driver_shp::__fuzz_wkb_roundtrip(data);
});
