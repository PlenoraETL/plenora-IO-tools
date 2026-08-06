#![no_main]
//! Coverage-guided sul reader Arrow IPC: è il formato di interscambio con
//! plenora-data-tools e l'unico pass-through, quindi schema e metadati di
//! contratto (`geoarrow.wkb`, CRS, versione) arrivano dal file senza
//! normalizzazione intermedia.
use libfuzzer_sys::fuzz_target;

mod harness;

fuzz_target!(|data: &[u8]| {
    let Some(file) = harness::spill(data, "input.arrow") else {
        return;
    };
    // Err = IPC invalido o contratto geometrico non conforme; un panic è un finding.
    let _ = harness::read_all(
        &driver_ipc::IpcDriver,
        file.path().to_path_buf(),
        &harness::read_options(),
    );
});
