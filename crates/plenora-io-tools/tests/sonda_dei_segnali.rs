//! Il secondo Ctrl+C esce subito, provato senza toccare il contratto pubblico.
//!
//! # Il problema, e perche' la prima soluzione era sbagliata
//!
//! La riga che chiama `std::process::exit` non e' osservabile da un test in
//! processo: terminerebbe il harness. Serve un processo che muoia davvero, e
//! serve sapere **quando** il primo segnale e' stato ricevuto -- altrimenti il
//! secondo parte a un istante scelto dal test, e quale dei due percorsi vince
//! dipende da quanto e' carica la macchina. Serve cioe' una barriera.
//!
//! Il primo tentativo l'aveva messa su `stderr`: una riga di conferma scritta
//! dal gestore. Funzionava, e rompeva il contratto -- `stderr` porta **un solo
//! documento JSON**, e un consumatore che lo legge con un parser sull'intero
//! flusso trovava una riga di testo davanti. Peggio: la sonda che avrebbe
//! dovuto accorgersene era stata riscritta per analizzare soltanto l'ultima
//! riga, cioe' resa compatibile con la riga di troppo invece di rifiutarla.
//!
//! Qui la barriera sta su un **canale privato della sonda**: un file il cui
//! percorso il figlio riceve in una variabile d'ambiente sua. Nessun flusso
//! pubblico lo vede, e il contratto della busta resta quello che era.
//!
//! # Che cosa esegue davvero
//!
//! Il figlio e' questo stesso binario di test, rieseguito con il filtro della
//! funzione fixture. Il gestore che installa e' **lo stesso** della CLI, perche'
//! `segnali.rs` e' incluso da qui e da `main.rs` -- lo stesso file, non una
//! copia. Una copia sarebbe potuta divergere, e questa sonda avrebbe continuato
//! a provare un gestore che non esiste piu'.

// Tutto questo file parla di segnali POSIX: la barriera, il secondo segnale, il
// codice d'uscita `128 + SIGINT`. Su Windows non c'e' l'equivalente di cio' che
// prova -- non un modo diverso di provarlo, proprio un'altra cosa -- e
// compilarne meta' lascerebbe costanti e funzioni che nessuno legge, che con
// `-D warnings` sono errori. Che il gestore compili su Windows lo dice gia'
// `main.rs`, che include lo stesso file.
#![cfg(unix)]

#[path = "../src/segnali.rs"]
mod segnali;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// La variabile che trasforma la funzione fixture in un processo che aspetta.
///
/// Senza, la fixture non fa niente e passa: gira nella suite come tutti gli
/// altri test, e non resta appesa.
const ACK: &str = "PLENORA_SONDA_SEGNALI_ACK";

/// Quanto si aspetta un riscontro dal figlio.
///
/// Trenta secondi, e non uno: il numero non descrive quanto ci mette il figlio
/// -- sono millisecondi -- ma quanto puo' metterci su una macchina carica, dove
/// gira l'intera suite. Un'attesa lunga non rallenta nulla quando le cose
/// funzionano, perche' si esce al primo riscontro.
const ATTESA: std::time::Duration = std::time::Duration::from_secs(30);
const INTERVALLO: std::time::Duration = std::time::Duration::from_millis(10);

/// Scrive un riscontro che il genitore possa attendere per **comparsa**.
///
/// Scritto a parte e poi rinominato: un file che appare vuoto e si riempie dopo
/// sarebbe una barriera che si apre troppo presto.
fn scrivi(percorso: &Path) {
    let parziale = percorso.with_extension("parziale");
    let mut file = std::fs::File::create(&parziale).expect("il file di riscontro si crea");
    file.write_all(b"x").expect("il riscontro si scrive");
    file.sync_all().expect("il riscontro raggiunge il disco");
    drop(file);
    std::fs::rename(&parziale, percorso).expect("il riscontro compare intero");
}

/// Il figlio della sonda: installa il gestore vero, dichiara, e aspetta.
///
/// Non e' una prova, e nella suite non fa niente: senza la variabile
/// d'ambiente ritorna subito. E' l'unico modo di avere un processo che installa
/// **questo** gestore senza aggiungere un binario al prodotto -- un `src/bin`
/// finirebbe negli artefatti di release, e una fixture di test non ci va.
#[test]
fn fixture_dei_segnali() {
    let Some(ack) = std::env::var_os(ACK) else {
        return;
    };
    let ack = PathBuf::from(ack);

    let token =
        segnali::installa_gestore_dei_segnali().expect("il primo gestore del processo si installa");
    // Il primo riscontro dice che il gestore e' installato: senza, un segnale
    // mandato troppo presto morirebbe sulla disposizione predefinita, e il
    // figlio se ne andrebbe senza passare dal gestore.
    scrivi(&ack.with_extension("pronto"));

    while !token.is_cancelled() {
        std::thread::sleep(INTERVALLO);
    }
    // Il token e' armato: il primo segnale e' arrivato **ed e' stato trattato**.
    // E' questa la barriera, non l'installazione.
    scrivi(&ack);

    // Da qui il figlio non fa niente e non muore da solo: se ne va soltanto per
    // il secondo segnale, che e' cio' che la sonda misura. Se uscisse per conto
    // proprio, il codice d'uscita non direbbe piu' niente.
    loop {
        std::thread::sleep(INTERVALLO);
    }
}

/// Il figlio della sonda, che non sopravvive alla sonda.
///
/// Senza, un `panic!` prima del secondo segnale lascerebbe il figlio in attesa
/// **per sempre**: aspetta apposta, e nessuno lo raccoglie. Un processo orfano
/// per corsa e' poco; una suite che gira in ciclo ne lascia uno per fallimento,
/// e il primo a rendersene conto sarebbe chi guarda la macchina, non chi legge
/// il test.
///
/// `Drop` gira anche durante lo srotolamento di un panico, quindi la pulizia
/// non dipende dal fatto che la sonda arrivi in fondo.
struct FiglioSorvegliato(std::process::Child);

impl Drop for FiglioSorvegliato {
    fn drop(&mut self) {
        // `kill` su un processo gia' morto e' un errore che qui non significa
        // niente: la sonda normale lo ha gia' raccolto con `wait`.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn attendi(percorso: &Path, figlio: &mut std::process::Child, cosa: &str) {
    let scadenza = std::time::Instant::now() + ATTESA;
    while std::time::Instant::now() < scadenza {
        if percorso.exists() {
            return;
        }
        if let Some(stato) = figlio.try_wait().expect("lo stato del figlio e' leggibile") {
            panic!("il figlio e' uscito con {stato:?} prima di dichiarare «{cosa}»");
        }
        std::thread::sleep(INTERVALLO);
    }
    panic!("«{cosa}» non e' arrivato entro {ATTESA:?} e il figlio e' ancora vivo");
}

fn segnala(pid: u32) {
    // Il `kill` della shell invece di una chiamata alla libc: il workspace vieta
    // `unsafe`, e il builtin fa cio' che serve senza pretendere che l'immagine
    // abbia `/bin/kill` -- che l'immagine di sviluppo, minimale, non ha.
    let esito = Command::new("sh")
        .arg("-c")
        .arg(format!("kill -INT {pid}"))
        .status()
        .expect("la shell si esegue");
    assert!(esito.success(), "kill -INT non riuscito");
}

/// Il secondo segnale fa uscire il processo con `128 + SIGINT`, e il primo no.
///
/// I due segnali prendono rami diversi dello stesso gestore, e la differenza
/// non e' osservabile dal codice d'uscita -- che sarebbe `130` in entrambi i
/// casi se la pipeline rientrasse. Qui il figlio **non ha una pipeline**:
/// aspetta e basta. Quindi il primo segnale non lo puo' far uscire, e se il
/// processo muore e' perche' il secondo ha preso il ramo dell'uscita.
///
/// La sonda verifica esplicitamente che il figlio sia vivo fra i due segnali:
/// e' l'affermazione che rende il risultato una proprieta' invece di una corsa.
#[test]
fn il_secondo_segnale_fa_uscire_il_processo_con_centotrenta() {
    let dir = tempfile::tempdir().unwrap();
    let ack = dir.path().join("armato");

    let mut sorvegliato = FiglioSorvegliato(
        Command::new(std::env::current_exe().expect("il binario di test ha un percorso"))
            .args(["--exact", "fixture_dei_segnali", "--nocapture"])
            .env(ACK, &ack)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("il figlio parte"),
    );
    let figlio = &mut sorvegliato.0;

    attendi(&ack.with_extension("pronto"), figlio, "gestore installato");
    segnala(figlio.id());
    attendi(&ack, figlio, "primo segnale trattato");

    assert!(
        figlio.try_wait().expect("stato leggibile").is_none(),
        "il primo segnale non deve far uscire un processo che non ha niente da annullare: \
         e' cio' che rende deterministico il secondo"
    );

    segnala(figlio.id());
    let stato = figlio.wait().expect("il figlio termina");
    assert_eq!(
        stato.code(),
        Some(segnali::EXIT_ANNULLATO),
        "il secondo segnale esce dal gestore con 128 + SIGINT"
    );
}

/// Un secondo gestore non si installa, e il rifiuto e' **tipizzato**.
///
/// # La decisione che questa sonda fissa
///
/// Il ramo esisteva e stampava un avviso testuale su `stderr`. Non era
/// raggiungibile da un uso normale -- la CLI installa il gestore una volta
/// sola -- ma se lo fosse stato avrebbe messo una riga di testo **davanti alla
/// busta**, rompendo il contratto per cui `stderr` porta un documento solo. E
/// l'avrebbe fatto proprio nel caso in cui l'avviso conta di piu': quando il
/// comando fallisce e lo staging resta sul disco.
///
/// Adesso e' un rifiuto tipizzato, leggibile da una macchina, e il comando non
/// parte. Il costo e' dichiarato: dove i segnali non si armano, la CLI si
/// rifiuta di lavorare.
///
/// La sonda raggiunge il ramo installando due volte: `ctrlc` ammette un gestore
/// per processo, e il secondo tentativo e' l'unico modo di far fallire
/// `set_handler` senza una piattaforma che non lo supporti.
#[test]
fn un_secondo_gestore_e_un_rifiuto_tipizzato_non_una_riga_di_testo() {
    let primo = segnali::installa_gestore_dei_segnali();
    assert!(primo.is_ok(), "il primo gestore del processo si installa");

    let Err(errore) = segnali::installa_gestore_dei_segnali() else {
        panic!("`ctrlc` ammette un gestore per processo: il secondo non si installa");
    };
    assert_eq!(errore.message, segnali::RIFIUTO_SEGNALI);
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::InvalidConfiguration,
        "e' l'ambiente a non permettere l'invocazione, non l'invocazione a essere sbagliata"
    );
    assert_eq!(
        errore.retry,
        plenora_io_model::RetryDisposition::Never,
        "riprovare non installa un gestore che il processo ha gia'"
    );
}
