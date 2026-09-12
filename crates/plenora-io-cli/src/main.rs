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

use plenora_io_cli::{
    busta_di_successo, con_identita, envelope_panico, installa_hook_silenzioso, radici, run,
    uscita_della_categoria, AIUTO, COMANDO_IGNOTO,
};
use plenora_io_model::ErrorCategory;
use serde_json::Value;

fn main() {
    // Per prima cosa, e prima che nasca un secondo thread: `set_var` muta
    // l'ambiente del processo, e le librerie native lo leggono pigramente.
    // Qualunque cosa qui sotto puo' gia' aprire un dataset.
    radici::radici_dell_artefatto();
    installa_hook_silenzioso();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        // `--help` e' l'unico esito che non e' una busta: e' testo per persone,
        // e CLI 2.0 non vincola il formato umano. Esce 0 su stdout come ogni
        // successo.
        Ok(("help", Ok(Value::Null))) => print!("{AIUTO}"),
        Ok((comando, Ok(corpo))) => println!("{}", busta_di_successo(comando, corpo)),
        // La busta d'errore esce su **stdout**, e stderr resta vuoto.
        //
        // Usciva su stderr, e stdout restava vuoto: un consumatore che legge
        // stdout -- cioe' quello che il contratto descrive -- non vedeva il
        // fallimento affatto. E' una selezione di stream, ed e' incompatibile
        // cambiarla: per questo appartiene alla major.
        Ok((comando, Err((exit, doc)))) => {
            println!("{}", con_identita(doc, comando));
            std::process::exit(exit);
        }
        // Un panico e' un errore `internal`, e la sua proiezione e' `70`.
        //
        // Usciva `2` su stderr, cioe' il codice della configurazione non valida:
        // diceva a chi automatizza «correggi la richiesta» davanti a un difetto
        // nostro. Il messaggio resta redatto e porta la sola impronta.
        Err(payload) => {
            println!(
                "{}",
                con_identita(envelope_panico(payload.as_ref()), COMANDO_IGNOTO)
            );
            std::process::exit(uscita_della_categoria(ErrorCategory::Internal));
        }
    }
}
