#![no_main]
//! Coverage-guided sul **percorso FileGDB reale**: apertura del dataset via
//! GDAL, catalogo, schema di ciascun layer e drenaggio delle righe.
//!
//! # Che cosa questo target non e'
//!
//! Non e' un fuzzer di GDAL. `libgdal.so` e' una libreria di sistema collegata
//! dinamicamente e **non strumentata**: non porta contatori di copertura, e il
//! fuzzer e' cieco dentro di essa. Cio' che guida le mutazioni e' la copertura
//! del **wrapper Rust**; cio' che gira dentro GDAL gira senza feedback.
//!
//! La conseguenza va detta, perche' e' facile presentarla al contrario: una
//! campagna verde qui dice che il percorso Rust regge input ostili e che GDAL
//! non e' stato portato a un crash **osservabile**, non che GDAL sia stato
//! esplorato. Il perimetro esatto di cio' che AddressSanitizer vede sta in
//! `assurance/registries/asan-filegdb.json`, ed e' misurato invece che
//! dichiarato.
//!
//! # Perche' qui non c'e' quasi niente
//!
//! La divisione dell'input e la materializzazione della `.gdb` vivono in
//! `driver-filegdb`, accanto al codice che esercitano: li' sono **provabili**, e
//! le sonde del driver le chiamano sulla stessa fixture per verificare che
//! raggiungano davvero il drenaggio. Una build che compila e un replay senza
//! crash non lo dimostrano.
use libfuzzer_sys::fuzz_target;

mod harness;

/// La fixture e' **nostra** e non viene dal fuzzer: sta nel binario, cosi' il
/// target non dipende dalla directory da cui viene lanciato.
static FIXTURE: &[u8] = include_bytes!("../fixtures/filegdb/citta.gdb.bundle");

fuzz_target!(|data: &[u8]| {
    let _ = driver_filegdb::__fuzz_leggi_gdb(FIXTURE, data, harness::read_options());
});
