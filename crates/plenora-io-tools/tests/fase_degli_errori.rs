//! La fase dichiarata dalla busta, sul percorso pubblico.
//!
//! ERR-003 dice che `phase` nomina «the last externally meaningful phase known
//! to have started». Gli analizzatori condivisi -- `parse_wkt_bounded`,
//! `decode_wkb`, `format_wkt` -- non sanno in quale passata girano: servono
//! l'inferenza, i loop di lettura e i writer, e la fase del loro costruttore e'
//! percio' un default. A dichiararla dev'essere lo stadio.
//!
//! Qui stanno i casi che la **riga di comando** raggiunge. Gli altri percorsi
//! corretti sono rami difensivi dietro `with_write_validation`, e le loro
//! regressioni vivono dentro i rispettivi crate.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const fn binario() -> &'static str {
    env!("CARGO_BIN_EXE_plenora-io")
}

fn esegui(argomenti: &[&str]) -> (Option<i32>, Value) {
    let esito = Command::new(binario())
        .args(argomenti)
        .output()
        .expect("il binario parte");
    let busta: Value = serde_json::from_slice(&esito.stdout).expect("la busta e' JSON");
    (esito.status.code(), busta)
}

/// Una sorgente CSV con una geometria lunga ma valida.
///
/// Il testo WKT misura poco piu' di quattrocento byte: sta comodamente dentro i
/// tetti di default, e serve solo a esistere sotto una soglia che poi stringo.
fn sorgente_con_wkt_lungo(percorso: &Path) -> usize {
    let vertici: Vec<String> = (0..100).map(|i| format!("{} {}", i % 9, i % 9)).collect();
    let linea = format!("LINESTRING({})", vertici.join(","));
    let mut testo = String::from("id,geometry\n");
    for i in 0..5 {
        writeln!(testo, "{i},\"{linea}\"").expect("la riga si compone");
    }
    std::fs::write(percorso, testo).expect("la sorgente si scrive");
    linea.len()
}

/// L'inferenza dell'XLSX gira dentro `open`, e lo dichiara.
///
/// # Come ci si arriva
///
/// Con l'unica asimmetria utile fra le due passate del driver: l'inferenza
/// **analizza** la cella WKT, il resto della pipeline no. Stringendo
/// `--max-wkb-cell-bytes` sotto la lunghezza del testo, il rifiuto arriva dove
/// il foglio viene esaminato per dedurne lo schema, cioe' durante
/// l'allestimento del reader.
///
/// # Che cosa fissa
///
/// `phase: prepare`. Valeva `validate`, perche' la fase la dichiarava
/// `parse_wkt_bounded`, che non sa se sta servendo l'inferenza o un writer.
/// `prepare` e non `probe` perche' e' lo stadio di `reader_busy` e
/// `projection_unsupported`, che nascono nello stesso `open`.
///
/// La corsa di controllo senza il tetto deve riuscire: senza, la prova non
/// distinguerebbe il rifiuto voluto da un file che non si legge comunque.
#[test]
fn l_inferenza_xlsx_dichiara_l_allestimento_del_reader() {
    let radice = tempfile::tempdir().expect("directory temporanea");
    let csv = radice.path().join("in.csv");
    let byte_wkt = sorgente_con_wkt_lungo(&csv);
    let xlsx: PathBuf = radice.path().join("foglio.xlsx");

    let (codice, busta) = esegui(&[
        "convert",
        csv.to_str().unwrap(),
        xlsx.to_str().unwrap(),
        "--from",
        "csv",
        "--to",
        "xls",
        "--assume-crs",
        "OGC:CRS84",
        "--in-opt",
        "wkt_column=geometry",
    ]);
    assert_eq!(
        codice,
        Some(0),
        "la sorgente XLSX si deve produrre: {busta}"
    );

    // Controllo: senza tetto la rilettura riesce.
    let libera = radice.path().join("libera.geojson");
    let (codice, busta) = esegui(&[
        "convert",
        xlsx.to_str().unwrap(),
        libera.to_str().unwrap(),
        "--from",
        "xls",
        "--to",
        "geojson",
        "--assume-crs",
        "OGC:CRS84",
        "--in-opt",
        "wkt_column=geometry",
    ]);
    assert_eq!(codice, Some(0), "senza tetto la rilettura riesce: {busta}");

    // La corsa in esame: il tetto sta sotto la lunghezza del testo.
    let tetto = byte_wkt / 2;
    let stretta = radice.path().join("stretta.geojson");
    let (codice, busta) = esegui(&[
        "convert",
        xlsx.to_str().unwrap(),
        stretta.to_str().unwrap(),
        "--from",
        "xls",
        "--to",
        "geojson",
        "--assume-crs",
        "OGC:CRS84",
        "--in-opt",
        "wkt_column=geometry",
        "--max-wkb-cell-bytes",
        &tetto.to_string(),
    ]);

    // Il codice d'uscita di una quota superata e' il suo, distinto da quello
    // del formato: cambiarlo non e' oggetto di questa correzione.
    assert_eq!(codice, Some(4), "il tetto deve rifiutare: {busta}");
    let errore = &busta["error"];
    assert_eq!(
        errore["phase"], "prepare",
        "l'inferenza gira dentro `open`, come `reader_busy`: {busta}"
    );
    // La correzione riguarda la sola fase: gli altri assi non si muovono.
    assert_eq!(errore["code"], "LIMIT_EXCEEDED", "{busta}");
    assert_eq!(errore["category"], "resource_limit", "{busta}");
    assert_eq!(errore["retry"]["kind"], "never", "{busta}");
    assert_eq!(errore["remote_effect"], "none", "{busta}");
    assert!(
        errore["message"]
            .as_str()
            .is_some_and(|m| m.contains(&byte_wkt.to_string())),
        "il messaggio deve nominare la dimensione vera della cella: {busta}"
    );

    assert!(!stretta.exists(), "la destinazione non deve esistere");
}
