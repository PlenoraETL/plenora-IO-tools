#![no_main]
//! Coverage-guided sul reader CSV: intestazione, inferenza di tipo a due
//! passate, adattatore WKT e costruzione dei batch. Il CRS è dichiarato
//! (ADR-IO 4), quindi la geometria arriva sempre dal contenuto del file.
use libfuzzer_sys::fuzz_target;

mod harness;

fuzz_target!(|data: &[u8]| {
    let Some(file) = harness::spill(data, "input.csv") else {
        return;
    };
    let path = file.path().to_path_buf();
    // Il driver rifiuta l'apertura senza una dichiarazione di geometria: le due
    // configurazioni coprono il ramo WKT e il ramo X/Y, che inferiscono tipi e
    // dimensioni in modo diverso.
    for options in harness::declared_geometry_read_options() {
        let _ = harness::read_all(&driver_csv::CsvDriver, path.clone(), &options);
    }
});
