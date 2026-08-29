//! `convert` regge un ingresso molto piu' grande del budget di memoria.
//!
//! # Perche' passa dal binario e non da una funzione
//!
//! La proprieta' in prova e' del **comando**, non di un helper: comprende il
//! parsing dei flag, la costruzione del budget unificato, l'adapter
//! operation-atomic con il suo spool, il writer e il publish. Una prova che
//! chiamasse `trasferisci_layer` direttamente verificherebbe la parte che ha
//! gia' le proprie sonde di sequenza in `main.rs`, e lascerebbe fuori
//! esattamente cio' che qui interessa: che la quota scelta dall'operatore
//! arrivi fino allo spool e che la conversione sopravviva.
//!
//! # Che cosa la prova dice e che cosa non dice
//!
//! Dice che una conversione con `--memory-bytes` quattro volte piu' piccolo
//! della sorgente arriva in fondo e conserva tutte le righe. **Non** misura il
//! picco di RSS del processo, e non pretende di farlo: misurarlo dentro un test
//! lo renderebbe dipendente dall'allocatore, dal sistema operativo e dagli
//! altri test in parallelo, cioe' rumoroso proprio dove serve un verdetto.
//!
//! Cio' che l'accumulo in RAM aveva di osservabile e deterministico e' la
//! **sequenza** di letture e scritture, provata dalle sonde di `main.rs`. Le
//! due prove sono complementari e nessuna sostituisce l'altra: la sequenza dice
//! che la CLI non trattiene piu' di un batch, questa dice che l'intera pipeline
//! resta in piedi quando la quota e' molto piu' piccola dei dati.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Righe della sorgente grande.
const RIGHE: usize = 400_000;

/// Righe della sorgente di controllo, che non deve essere grande per dire cio'
/// che dice.
const RIGHE_CONTROLLO: usize = 5_000;

/// Riempimento per riga, in caratteri.
///
/// Il volume si compra con righe **grasse** invece che con righe numerose: il
/// costo di parsing per byte di un campo lungo e' molto piu' basso di quello di
/// una riga in piu', e la prova deve stare comodamente dentro la deadline di
/// pipeline anche in build di debug.
const RIEMPIMENTO: usize = 320;

/// Quota di memoria della pipeline durante la prova.
///
/// Non puo' scendere a piacere, ed e' bene che la prova lo dica invece di
/// sembrare una scelta arbitraria. Due vincoli la governano insieme:
///
/// * un batch materializzato deve stare nella prenotazione presa per lui, e
///   `BatchTarget::default()` punta a **8 MiB**;
/// * lo spool migra su file quando l'occupato supera la **meta'** della
///   capacita' di memoria, quindi finche' non ha migrato la memoria libera puo'
///   essere solo meta' della quota.
///
/// Insieme pretendono `memory_bytes > 2 x 8 MiB`. Sotto quel valore la
/// conversione fallisce con `LIMIT_EXCEEDED` — «batch materializzato oltre la
/// quota prenotata» — che e' un rifiuto corretto e non dice niente sul
/// buffering. Trentadue MiB lasciano il margine di un batch pieno da entrambi i
/// lati.
const MEMORY_BYTES: u64 = 32 * 1024 * 1024;

/// `PipelineLimits::validate` pretende `max_wkb_cell_bytes <= memory_bytes`, e
/// il default per cella e' 64 MiB: senza abbassarlo insieme alla memoria la
/// conversione fallirebbe in validazione dei limiti invece che dire qualcosa
/// sullo spool.
const CELL_BYTES: u64 = 64 * 1024;

/// `driver-csv` pretende `assume_crs` all'apertura, sempre: un CSV non porta
/// CRS e il driver non ne inventa uno.
const CRS: &str = "EPSG:4326";

/// La colonna WKT, dichiarata invece che indovinata dal driver.
const OPZIONE_WKT: &str = "wkt_column=geom";

const fn binario() -> &'static str {
    env!("CARGO_BIN_EXE_plenora-io")
}

/// Sorgente CSV con una colonna numerica, una testuale lunga e una geometria.
///
/// La geometria non e' decorativa: `driver-csv` non apre un CSV senza sapere
/// quale colonna sia la geometria. Un punto per riga tiene il costo per cella
/// molto sotto `CELL_BYTES`, cosi' il tetto per cella non entra nel discorso
/// che questa prova vuole fare, che riguarda il buffering.
///
/// Scrive con un `BufWriter` invece di comporre una `String`: il file e' grande
/// quanto basta perche' tenerlo tutto in memoria dentro il test sarebbe la
/// stessa disattenzione che il test esiste per rilevare.
fn scrivi_sorgente(percorso: &Path, righe: usize) -> u64 {
    let riempimento = "x".repeat(RIEMPIMENTO);
    let file = std::fs::File::create(percorso).expect("sorgente creata");
    let mut uscita = BufWriter::new(file);
    writeln!(uscita, "id,etichetta,geom").expect("intestazione scritta");
    for indice in 0..righe {
        let x = indice % 360;
        let y = indice % 180;
        writeln!(uscita, "{indice},{riempimento},POINT({x}.5 {y}.5)").expect("riga scritta");
    }
    uscita.flush().expect("sorgente completata");
    drop(uscita);
    std::fs::metadata(percorso)
        .expect("sorgente misurabile")
        .len()
}

fn percorsi(directory: &Path) -> (PathBuf, PathBuf) {
    (
        directory.join("ingresso.csv"),
        directory.join("uscita.arrow"),
    )
}

/// Esegue `convert`, con la quota stretta oppure con i default.
fn converti(ingresso: &Path, uscita: &Path, quota_stretta: bool) -> (bool, String, String) {
    let mut comando = Command::new(binario());
    comando
        .arg("convert")
        .arg(ingresso)
        .arg(uscita)
        .arg("--assume-crs")
        .arg(CRS)
        .arg("--in-opt")
        .arg(OPZIONE_WKT);
    if quota_stretta {
        comando
            .arg("--memory-bytes")
            .arg(MEMORY_BYTES.to_string())
            .arg("--max-wkb-cell-bytes")
            .arg(CELL_BYTES.to_string());
    }
    let output = comando.output().expect("il binario si esegue");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn documento(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout).expect("stdout e' il documento JSON del comando")
}

#[test]
fn convert_regge_un_ingresso_molto_piu_grande_della_quota_di_memoria() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let (ingresso, uscita) = percorsi(temporanea.path());
    let byte_sorgente = scrivi_sorgente(&ingresso, RIGHE);

    // La premessa della prova, verificata invece che assunta: se un giorno
    // `RIGHE` o `RIEMPIMENTO` cambiassero fino a far stare la sorgente dentro
    // la quota, il test resterebbe verde senza piu' provare niente.
    assert!(
        byte_sorgente > MEMORY_BYTES * 4,
        "sorgente da {byte_sorgente} byte: non e' abbastanza piu' grande della quota di {MEMORY_BYTES}"
    );

    let (riuscita, stdout, stderr) = converti(&ingresso, &uscita, true);
    assert!(
        riuscita,
        "convert fallito sotto quota stretta.\nstdout: {stdout}\nstderr: {stderr}"
    );

    let doc = documento(&stdout);
    assert_eq!(doc["status"], "ok");
    assert_eq!(doc["total_rows"], RIGHE);
    assert_eq!(doc["publish_outcome"], "published");
    assert!(uscita.is_file(), "la destinazione non e' stata pubblicata");
}

/// La stessa conversione con la quota di default, su una sorgente piccola.
///
/// Serve a distinguere «la pipeline funziona» da «la pipeline funziona **anche**
/// sotto quota stretta»: senza questo confronto un rosso della prova precedente
/// non direbbe se il difetto sta nel buffering o nel percorso CSV → Arrow IPC.
#[test]
fn la_stessa_conversione_riesce_anche_con_la_quota_di_default() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let (ingresso, uscita) = percorsi(temporanea.path());
    scrivi_sorgente(&ingresso, RIGHE_CONTROLLO);

    let (riuscita, stdout, stderr) = converti(&ingresso, &uscita, false);
    assert!(
        riuscita,
        "convert fallito con i default.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(documento(&stdout)["total_rows"], RIGHE_CONTROLLO);
}
