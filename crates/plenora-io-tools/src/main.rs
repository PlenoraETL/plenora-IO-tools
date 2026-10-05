//! Il binario: apre il processo, e nient'altro.
//!
//! # Perche' la libreria e il binario sono separati
//!
//! Il profilo io-tools pretende una superficie Rust pubblica con una mappatura
//! versionata verso le operazioni, verificata da un consumatore che importa
//! **solo** gli export documentati. Finche' le operazioni vivevano dentro
//! `main.rs` quella superficie non esisteva: un binario non si importa.
//!
//! Ora esistono in `lib.rs`, e questo file e' cio' che resta di specifico del
//! processo -- le radici dell'artefatto, il gancio sui panici, la scelta del
//! flusso e la proiezione del codice d'uscita. E' il **binding** CLI
//! dell'operazione, non l'operazione.
//!
//! Che le due superfici siano equivalenti non e' una promessa da verificare:
//! chiamano la stessa funzione.

use plenora_io_tools::{
    installa_hook_silenzioso, radici, run, uscita_del_panico, uscita_del_processo,
};

fn main() {
    // Per prima cosa, e prima che nasca un secondo thread: `set_var` muta
    // l'ambiente del processo, e le librerie native lo leggono pigramente.
    // Qualunque cosa qui sotto puo' gia' aprire un dataset.
    radici::radici_dell_artefatto();
    installa_hook_silenzioso();
    // La proiezione -- che cosa va su stdout, con quale codice -- sta in
    // `uscita_del_processo`, condivisa con il modulo nativo dello SDK Python.
    // Qui resta cio' che e' del processo: stampare e uscire.
    //
    // Su stdout esce **sempre** la busta, anche d'errore, e stderr resta
    // vuoto: un consumatore che legge stdout -- cioe' quello che il contratto
    // descrive -- vede entrambi gli esiti. Un panico e' un errore `internal`,
    // e la sua proiezione e' `70`, non quella della configurazione non valida.
    let (uscita, testo) = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(esito) => uscita_del_processo(esito),
        Err(payload) => uscita_del_panico(payload.as_ref()),
    };
    print!("{testo}");
    if uscita != 0 {
        std::process::exit(uscita);
    }
}
