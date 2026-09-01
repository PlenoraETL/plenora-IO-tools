//! Impone all'ambiente del processo le radici che l'artefatto porta con se'.
//!
//! Il **piano** vive in `plenora_io_core::radici`, e questo modulo lo applica
//! all'ambiente. E' una delle due vie, e da sola non basta:
//!
//! - su Linux funziona, perche' `setenv` aggiorna `environ` ed e' quello che
//!   `getenv` legge, anche dalle librerie native;
//! - su Windows **non** basta. `std::env::set_var` chiama
//!   `SetEnvironmentVariableW`, che aggiorna il blocco d'ambiente del processo;
//!   il runtime C ne tiene una copia, e `getenv` legge quella. GDAL e PROJ
//!   chiamano `getenv`, e la variabile impostata da Rust non la vedono.
//!
//! L'altra via -- le config option di GDAL -- sta in `driver-filegdb`, dove
//! GDAL c'e'. Questa resta perche' su Linux e' sufficiente, perche' vale anche
//! per le librerie che non passano da GDAL, e perche' un processo figlio
//! erediterebbe l'ambiente e non le config option.

use plenora_io_core::radici::piano_del_processo;

/// Impone all'ambiente le radici dell'artefatto. Da chiamare per prima cosa in
/// `main`, prima che esista un secondo thread.
pub fn radici_dell_artefatto() {
    for (variabile, valore) in piano_del_processo() {
        std::env::set_var(variabile, valore);
    }
}
