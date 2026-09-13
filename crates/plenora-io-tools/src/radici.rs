//! Impone le radici che l'artefatto porta con se'.
//!
//! Il **piano** vive in `plenora_io_core::radici`, in un posto solo. Qui si
//! applica, e l'applicazione non e' una sola cosa: nessuno dei meccanismi
//! disponibili copre tutti i destinatari.
//!
//! # L'ambiente
//!
//! Su Linux basta: `setenv` aggiorna `environ`, ed e' quello che `getenv` legge
//! anche dalle librerie native.
//!
//! Su Windows **non** basta. `std::env::set_var` chiama
//! `SetEnvironmentVariableW`, che aggiorna il blocco d'ambiente del processo; il
//! runtime C ne tiene una copia, e `getenv` legge quella. GDAL e PROJ chiamano
//! `getenv`, e la variabile impostata da Rust non la vedono.
//!
//! Resta comunque, perche' vale su Linux, perche' vale per le librerie che non
//! passano da GDAL, e perche' un processo figlio eredita l'ambiente e non altro.
//!
//! # Dentro GDAL, e dentro PROJ
//!
//! `driver_filegdb::radici::applica` copre i due destinatari che l'ambiente non
//! raggiunge su Windows, con **due** meccanismi diversi: le config option per
//! GDAL, `OSRSetPROJSearchPaths` per PROJ, che quella tabella non la legge.
//!
//! Si chiama **qui**, all'avvio, e non all'apertura di un dataset: il finder dei
//! dati di GDAL si inizializza al primo uso, e la risoluzione del CRS avviene in
//! validazione -- prima che un driver apra qualcosa. Applicarle dentro
//! `open` arrivava dopo il momento in cui servivano.

use plenora_io_core::radici::piano_del_processo;

/// Impone le radici dell'artefatto. Da chiamare per prima cosa in `main`, prima
/// che esista un secondo thread e prima di qualunque uso delle librerie native.
pub fn radici_dell_artefatto() {
    for (variabile, valore) in piano_del_processo() {
        std::env::set_var(variabile, valore);
    }
    #[cfg(feature = "gdal-backend")]
    driver_filegdb::radici::applica();
}
