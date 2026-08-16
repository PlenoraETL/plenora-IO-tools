#![no_main]
//! Coverage-guided sul percorso di **scrittura**: Arrow IPC in ingresso,
//! GeoPackage in uscita.
//!
//! Arrow IPC è pass-through, quindi lo schema che arriva al writer è
//! interamente controllato dall'input: tipi, nullabilità, nomi di colonna e di
//! layer, metadati di contratto e CRS. È l'unico modo per esercitare offline la
//! validazione di capability, la coercizione Arrow → SQL, la generazione del
//! DDL della feature table, la registrazione dell'SRS e il publish atomico con
//! contratti che non siano quelli sintetizzati dai test.
use libfuzzer_sys::fuzz_target;

mod harness;

fuzz_target!(|data: &[u8]| {
    let Some((input, output)) = harness::spill_with_output(data, "input.arrow", "output.gpkg")
    else {
        return;
    };
    // Err = IPC invalido, contratto non scrivibile in GeoPackage o limiti
    // superati; un panic è un finding.
    let _ = harness::convert(
        &driver_ipc::IpcDriver,
        input.path().to_path_buf(),
        &driver_gpkg::GpkgDriver,
        output,
    );
});
