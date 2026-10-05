//! Il modulo nativo dello SDK Python: la CLI eseguita nel processo.
//!
//! # Che cosa espone, e perche' cosi' poco
//!
//! Una funzione, `esegui`, che prende gli argomenti della riga di comando e
//! rende il codice d'uscita e lo stdout che il binario `plenora-io` avrebbe
//! prodotto. Lo SDK Python resta un client del **protocollo v2**: costruisce
//! gli stessi argomenti, decodifica le stesse buste, solleva gli stessi
//! errori. Cambia soltanto dove gira il comando -- in un thread di questo
//! processo invece che in un processo figlio -- e quindi che non serve un
//! binario installato accanto.
//!
//! E' la scelta che tiene le due vie equivalenti per costruzione: il dispatch
//! e' `plenora_io_tools::esegui`, la proiezione su codice e stdout e'
//! `plenora_io_tools::uscita_del_processo`, e il binario chiama le stesse due
//! funzioni. Un binding che esponesse le operazioni una per una avrebbe un
//! secondo vocabolario da tenere allineato al protocollo.
//!
//! # Ctrl-C e scadenza
//!
//! Il comando gira in un thread suo, e il thread di Python aspetta il
//! risultato a intervalli brevi senza tenere il GIL. Fra un intervallo e
//! l'altro guarda i segnali. Se il gestore Python di un segnale solleva -- il
//! Ctrl-C predefinito solleva `KeyboardInterrupt` -- il comando si annulla in
//! modo cooperativo, come il binario al primo `SIGINT`, si **attende** che si
//! fermi (staging e spool cadono, nessuna pubblicazione), e poi l'eccezione
//! del gestore risale a chi ha chiamato, quella e non un'altra. Inghiottirla
//! per rendere la busta `CANCELLED` toglieva a Python la sua semantica dei
//! segnali: un gestore installato dall'applicazione non arrivava mai. Il
//! secondo `SIGINT`, che il binario usa per uscire subito, qui non ha
//! equivalente: un thread non si uccide, e il comando si attende fino al
//! proprio punto di verifica.
//!
//! Un `timeout` passato dallo SDK fa la stessa cosa allo scadere, e lo dice:
//! il terzo elemento del risultato e' `True`, e lo SDK solleva il proprio
//! errore di timeout invece di leggere la busta come un esito qualunque.

use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use plenora_io_model::CancellationToken;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

/// Ogni quanto il thread di Python torna a guardare segnali e scadenza.
///
/// Abbastanza breve da rendere un Ctrl-C immediato per chi lo batte, abbastanza
/// lungo da non pesare su un comando che dura minuti.
const INTERVALLO: Duration = Duration::from_millis(50);

/// Esegue un comando della CLI nel processo.
///
/// Rende `(codice_d_uscita, stdout, scaduto)`: i primi due sono quelli che il
/// binario avrebbe prodotto con gli stessi argomenti, il terzo dice se il
/// comando e' stato annullato perche' e' scaduto `timeout` (in secondi).
///
/// # Errors
///
/// `RuntimeError` se il thread del comando non parte o si interrompe senza
/// rendere un esito. L'eccezione sollevata dal gestore Python di un segnale
/// mentre il comando gira, dopo che il comando si e' fermato. Un errore del
/// comando non e' un'eccezione: e' una busta d'errore con il suo codice, come
/// per il binario.
#[pyfunction]
#[pyo3(signature = (argomenti, timeout=None))]
// PyO3 consegna gli argomenti per valore, e il thread del comando li deve
// possedere: un prestito non attraversa `spawn`.
#[allow(clippy::needless_pass_by_value)]
fn esegui(
    py: Python<'_>,
    argomenti: Vec<String>,
    timeout: Option<f64>,
) -> PyResult<(i32, String, bool)> {
    let limite = timeout
        .map(|secondi| {
            Duration::try_from_secs_f64(secondi)
                .map_err(|_| PyRuntimeError::new_err("timeout non rappresentabile"))
        })
        .transpose()?;
    let cancellazione = CancellationToken::new();
    let del_comando = cancellazione.clone();
    let (invia, ricevi) = mpsc::channel();
    std::thread::Builder::new()
        .name("plenora-io-comando".to_owned())
        .spawn(move || {
            let esito = std::panic::catch_unwind(AssertUnwindSafe(|| {
                plenora_io_tools::uscita_del_processo(plenora_io_tools::esegui(
                    &argomenti,
                    &del_comando,
                ))
            }))
            .unwrap_or_else(|payload| plenora_io_tools::uscita_del_panico(payload.as_ref()));
            // Se Python non aspetta piu', non c'e' nessuno a cui dirlo.
            let _ = invia.send(esito);
        })
        .map_err(|_| PyRuntimeError::new_err("il thread del comando non e' partito"))?;

    let inizio = Instant::now();
    let mut scaduto = false;
    // L'eccezione di un gestore di segnale, tenuta finche' il comando non si
    // e' fermato: risale dopo, non al posto dell'attesa.
    let mut dal_segnale: Option<PyErr> = None;
    let mut ricevi = ricevi;
    loop {
        // Il ricevitore entra ed esce dalla chiusura per valore: e' `Send` ma
        // non `Sync`, e senza GIL si puo' portare con se' soltanto cio' che
        // si possiede.
        let (indietro, atteso) = py.detach(move || {
            let atteso = ricevi.recv_timeout(INTERVALLO);
            (ricevi, atteso)
        });
        ricevi = indietro;
        match atteso {
            Ok(esito) => {
                return match dal_segnale {
                    // Il comando si e' fermato (o era gia' finito): ora
                    // l'eccezione del gestore risale, quella e non un'altra.
                    Some(errore) => Err(errore),
                    None => Ok((esito.0, esito.1, scaduto)),
                };
            }
            Err(RecvTimeoutError::Timeout) => {
                if cancellazione.is_cancelled() {
                    continue;
                }
                // Il gestore Python dei segnali gira qui. Se solleva, il
                // comando si annulla e l'eccezione si tiene per dopo.
                if let Err(errore) = py.check_signals() {
                    dal_segnale = Some(errore);
                    cancellazione.cancel();
                } else if limite.is_some_and(|limite| inizio.elapsed() >= limite) {
                    scaduto = true;
                    cancellazione.cancel();
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(PyRuntimeError::new_err(
                    "il comando si e' interrotto senza rendere un esito",
                ))
            }
        }
    }
}

/// Il modulo `plenora_io._native`.
///
/// # Errors
///
/// Quelli di `PyO3` nel registrare funzione e attributi.
#[pymodule]
fn _native(modulo: &Bound<'_, PyModule>) -> PyResult<()> {
    // Lo stesso gancio silenzioso del binario: il messaggio di un panico puo'
    // contenere dati letti dal file, e il gancio predefinito lo scriverebbe su
    // stderr. Il panico arriva comunque, come busta redatta con la sola
    // impronta. Il gancio e' quello di questa libreria, non dell'interprete.
    plenora_io_tools::installa_hook_silenzioso();
    modulo.add_function(wrap_pyfunction!(esegui, modulo)?)?;
    modulo.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
