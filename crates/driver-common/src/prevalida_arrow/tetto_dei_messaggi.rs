//! Il tetto `MAX_BLOCCHI` sui messaggi di un flusso Arrow.
//!
//! # Perche' la prova non c'era, e perche' ora c'e'
//!
//! Non c'era, ed era dichiarato: raggiungere il tetto chiede piu' di un
//! milione di messaggi **decodificabili**, e la suite della CLI avrebbe
//! dovuto generarli a ogni corsa per esercitare un fondo di sicurezza.
//! Che sia cosi' difficile da raggiungere e' anche cio' che lo rende un
//! fondo: su quasi ogni input malformato scatta prima un'altra guardia.
//!
//! Ma «costoso» non e' «impossibile», e un fondo che nessuno ha mai visto
//! scattare e' un fondo di cui nessuno sa se c'e'. La prova sta qui e non
//! fra quelle della CLI perche' esercita **questa** funzione: passare dal
//! binario aggiungerebbe il costo di ogni strato in mezzo senza aggiungere
//! niente a cio' che si vuole fissare.
//!
//! # Il costo, misurato e non stimato
//!
//! Il messaggio piu' piccolo disponibile senza nuove dipendenze e' quello
//! di schema che `StreamWriter` produce per una colonna: **128 byte**. Il
//! flusso che supera il tetto misura percio' **128 MiB**, generati in
//! circa 60 ms e scanditi in circa 3,5 s. Due flussi di quella misura --
//! al tetto e oltre -- costano circa sette secondi.
//!
//! La fixture e' **generata**, non versionata: nel repository non entra
//! nessun megabyte. Il caso «sotto» usa dieci messaggi, perche' fra dieci e
//! un milione meno uno la guardia dice la stessa cosa, e pagare due volte
//! per la stessa affermazione non la rende piu' vera.

use std::io::Write as _;
use std::sync::Arc;

/// Il messaggio di schema che il writer produce, senza il marcatore finale.
///
/// Si ripete identico: al prevalidatore interessa che ogni messaggio
/// decodifichi e dichiari il proprio corpo, non che siano diversi fra loro.
fn messaggio_di_schema() -> Vec<u8> {
    let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
        "a",
        arrow_schema::DataType::Int8,
        false,
    )]));
    let mut byte = Vec::new();
    let mut scrittore =
        arrow_ipc::writer::StreamWriter::try_new(&mut byte, &schema).expect("il writer parte");
    scrittore.finish().expect("il flusso si chiude");
    // Gli ultimi otto byte sono il marcatore di fine flusso, che va scritto
    // una volta sola e in fondo.
    byte.truncate(byte.len() - 8);
    byte
}

/// Scrive un flusso di `quanti` messaggi e ne restituisce l'esito.
///
/// Il file sta in una directory temporanea con un nome che dice quanti
/// messaggi porta: due casi che girassero sullo stesso percorso si
/// sovrascriverebbero a vicenda sotto `cargo test`, che esegue in parallelo.
fn esito_con(quanti: usize) -> Result<(), String> {
    let modello = messaggio_di_schema();
    let percorso = std::env::temp_dir().join(format!("plenora-tetto-{quanti}.arrows"));
    {
        let file = std::fs::File::create(&percorso).expect("il file si crea");
        let mut uscita = std::io::BufWriter::with_capacity(1 << 20, file);
        for _ in 0..quanti {
            uscita.write_all(&modello).expect("il messaggio si scrive");
        }
        // Il marcatore di fine flusso non e' otto zeri: e' il prefisso di
        // continuazione seguito da una lunghezza nulla. Scriverlo a zero
        // faceva parlare la guardia del prefisso, cioe' un'altra -- ed e'
        // esattamente l'errore che queste prove esistono per distinguere.
        uscita
            .write_all(&[0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0])
            .expect("il marcatore si scrive");
        uscita.flush().expect("il buffer si svuota");
    }
    let esito = super::valida_flusso_ipc("prova", &percorso);
    std::fs::remove_file(&percorso).ok();
    esito.map_err(|errore| errore.message)
}

/// Dieci messaggi: il tetto non c'entra, e la guardia non deve parlare.
#[test]
fn sotto_il_tetto_la_guardia_non_parla() {
    let esito = esito_con(10);
    assert!(
        esito.is_ok(),
        "dieci messaggi di schema formano un flusso valido: {esito:?}"
    );
}

/// Esattamente al tetto: il confronto e' `>`, quindi ancora dentro.
///
/// E' il caso che distingue un tetto da un tetto spostato di uno. Se la
/// guardia diventasse `>=`, questa prova diventerebbe rossa e l'altra
/// resterebbe verde: da sole non basterebbero ne' l'una ne' l'altra.
#[test]
fn al_tetto_la_guardia_non_parla_ancora() {
    let esito = esito_con(super::MAX_BLOCCHI);
    assert!(
        esito.is_ok(),
        "al tetto il flusso e' ancora dentro: {esito:?}"
    );
}

/// Uno oltre: la guardia parla, e si verifica **quale**.
///
/// «Rifiutato» non basterebbe. Un flusso da 128 MiB puo' essere rifiutato
/// da qualunque altra guardia -- una lunghezza, un corpo oltre la fine --
/// e la prova resterebbe verde mentre il tetto che pretende di fissare e'
/// sparito. Il messaggio dice chi ha parlato.
#[test]
fn oltre_il_tetto_il_flusso_e_rifiutato() {
    let esito = esito_con(super::MAX_BLOCCHI + 1);
    let Err(messaggio) = esito else {
        panic!("un flusso oltre il tetto e' stato accettato");
    };
    assert!(
        messaggio.contains("troppi messaggi nel flusso Arrow"),
        "rifiutato dalla guardia sbagliata: «{messaggio}»"
    );
}
