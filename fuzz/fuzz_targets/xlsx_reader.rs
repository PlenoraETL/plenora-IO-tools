#![no_main]
//! Coverage-guided sul reader XLSX: contenitore ZIP (incluso il gate sul
//! rapporto di decompressione), XML del foglio via calamine, inferenza di tipo
//! sulle celle e adattatore WKT. Un `.xlsx` è un archivio: la superficie
//! comprende sia il contenitore sia il contenuto.
use libfuzzer_sys::fuzz_target;

mod harness;

fuzz_target!(|data: &[u8]| {
    let Some(file) = harness::spill(data, "input.xlsx") else {
        return;
    };
    let path = file.path().to_path_buf();
    for options in harness::declared_geometry_read_options() {
        let _ = harness::read_all(&driver_xls::XlsDriver, path.clone(), &options);
    }
});
