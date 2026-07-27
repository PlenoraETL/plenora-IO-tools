#![no_main]
//! Coverage-guided sul parser KML e sulla conversione diretta KML→WKB XY/XYZ.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Err = KML invalido o non rappresentabile; un panic è un finding.
    let _ = driver_kml::__fuzz_read_kml(data);
});
