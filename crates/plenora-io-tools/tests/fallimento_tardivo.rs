//! Che cosa resta quando l'operazione fallisce **dopo** aver lavorato.
//!
//! # La lacuna che queste prove chiudono
//!
//! M1 aveva registrato che una sola prova fissava il momento del fallimento --
//! `sigint_annulla_la_conversione_e_non_lascia_staging`, che attende la
//! comparsa dello staging prima di mandare il segnale -- e per un solo modo di
//! fallire. Budget esaurito e errore dei dati a metà stream non avevano una
//! prova che ne fissasse il momento.
//!
//! # Che cosa si verifica, e su quale superficie
//!
//! Tutto passa dalla **riga di comando**, cioè dalla superficie che un
//! consumatore osserva: la busta d'errore coi suoi quattro assi, la
//! destinazione, e la directory che la contiene. Nulla qui guarda dentro il
//! processo.
//!
//! # Che cosa queste prove **non** possono attestare
//!
//! Che la scrittura fosse cominciata, se non nel caso in cui la busta lo dica
//! da sé. Per il budget lo dice: il messaggio nomina i byte prodotti, e quel
//! numero si può conoscere solo misurando uno staging già scritto. Per l'errore
//! dei dati a riga 15.000 la superficie pubblica non espone quanto sia stato
//! fatto, e la prova verifica gli invarianti -- errore tipizzato, nessuna
//! destinazione, nessun residuo -- **senza** affermare il momento. Dirlo è
//! parte del valore della prova: una che affermasse il momento senza poterlo
//! osservare sarebbe peggio di una che tace.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn binario() -> &'static str {
    env!("CARGO_BIN_EXE_plenora-io")
}

/// Una sorgente CSV con `righe` punti, e un guasto WKT alla riga indicata.
///
/// Il guasto va **oltre il primo batch**, o il fallimento avverrebbe prima che
/// il writer esista e la prova misurerebbe un altro percorso.
fn sorgente(percorso: &Path, righe: usize, guasto: Option<usize>) {
    let mut testo = String::from("id,nome,geometry\n");
    for i in 0..righe {
        if Some(i) == guasto {
            testo.push_str(&format!("{i},rotto,POINT(non un numero)\n"));
        } else {
            testo.push_str(&format!("{i},n{i},POINT({i} {i})\n"));
        }
    }
    std::fs::write(percorso, testo).expect("la sorgente si scrive");
}

fn converti(ingresso: &Path, uscita: &Path, extra: &[&str]) -> (Option<i32>, Value) {
    let esito = Command::new(binario())
        .arg("convert")
        .arg(ingresso)
        .arg(uscita)
        .args(["--from", "csv", "--to", "geojson"])
        .args(["--assume-crs", "OGC:CRS84"])
        .args(["--in-opt", "wkt_column=geometry"])
        .args(extra)
        .output()
        .expect("il binario parte");
    let busta: Value = serde_json::from_slice(&esito.stdout).expect("la busta e' JSON");
    (esito.status.code(), busta)
}

/// Nella directory non resta niente oltre a cio' che ci si aspetta.
///
/// Lo staging vive accanto alla destinazione -- deve, per poter essere
/// rinominato atomicamente -- quindi un residuo si vede qui e non altrove.
fn contenuto(directory: &Path) -> Vec<String> {
    let mut nomi: Vec<String> = std::fs::read_dir(directory)
        .expect("la directory si legge")
        .map(|v| v.expect("voce").file_name().to_string_lossy().into_owned())
        .collect();
    nomi.sort();
    nomi
}

fn ambiente(righe: usize, guasto: Option<usize>) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let radice = tempfile::tempdir().expect("directory temporanea");
    let ingresso = radice.path().join("in.csv");
    let uscita = radice.path().join("uscita.geojson");
    sorgente(&ingresso, righe, guasto);
    (radice, ingresso, uscita)
}

/// Il budget d'uscita si esaurisce **a scrittura finita**, e non resta niente.
///
/// E' l'unico dei tre casi in cui la busta attesta il lavoro svolto: il
/// messaggio nomina i byte prodotti, e quel numero viene dalla dimensione di
/// uno staging che esiste. Un rifiuto preventivo non potrebbe conoscerlo.
#[test]
fn il_budget_d_uscita_esaurito_non_lascia_destinazione_ne_residui() {
    let (radice, ingresso, uscita) = ambiente(2_000, None);

    // Prima la corsa senza limite, che fissa quanto l'uscita misura davvero.
    let piena = radice.path().join("piena.geojson");
    let (codice, busta) = converti(&ingresso, &piena, &[]);
    assert_eq!(
        codice,
        Some(0),
        "la corsa di riferimento deve riuscire: {busta}"
    );
    let prodotti = busta["result"]["bytes_written"]
        .as_u64()
        .expect("la corsa riuscita dichiara i byte scritti");
    std::fs::remove_file(&piena).expect("la corsa di riferimento si rimuove");

    // Poi la stessa conversione sotto un tetto che quella misura supera.
    let tetto = prodotti / 2;
    let (codice, busta) = converti(
        &ingresso,
        &uscita,
        &["--max-output-bytes", &tetto.to_string()],
    );

    assert_eq!(codice, Some(4), "il codice d'uscita del limite: {busta}");
    let errore = &busta["error"];
    assert_eq!(errore["code"], "LIMIT_EXCEEDED", "{busta}");
    assert_eq!(errore["category"], "resource_limit", "{busta}");
    assert_eq!(errore["retry"]["kind"], "never", "{busta}");
    assert_eq!(
        errore["remote_effect"], "none",
        "niente e' uscito dal processo: {busta}"
    );
    // La fase e' `commit`, e non `validate`: ERR-003 dice che `phase` nomina
    // l'ultima fase esternamente significativa **iniziata**, e qui la lettura
    // e' avvenuta, la scrittura e' finita e si sta pubblicando. Era `validate`
    // fino a questa correzione, e diceva a chi legge che l'operazione non
    // aveva cominciato.
    assert_eq!(errore["phase"], "commit", "{busta}");

    // L'attestazione del lavoro svolto: il messaggio nomina i byte **prodotti**,
    // che coincidono con quelli della corsa riuscita. Solo uno staging scritto
    // per intero puo' dare quel numero.
    let messaggio = errore["message"].as_str().unwrap_or_default();
    assert!(
        messaggio.contains(&prodotti.to_string()),
        "il messaggio deve nominare i byte prodotti ({prodotti}), cioe' attestare \
         che la scrittura e' avvenuta prima del rifiuto: {messaggio}"
    );

    assert!(!uscita.exists(), "la destinazione non deve esistere");
    assert_eq!(
        contenuto(radice.path()),
        vec!["in.csv".to_owned()],
        "nella directory non deve restare uno staging"
    );
}

/// Un guasto dei dati oltre il primo batch: errore tipizzato, e niente resta.
///
/// **Non** attesta che la scrittura fosse cominciata: la superficie pubblica non
/// espone quanto sia stato fatto, e affermarlo qui sarebbe dedurlo dal numero di
/// riga. Cio' che la prova fissa sono gli invarianti che valgono comunque.
#[test]
fn un_guasto_dei_dati_a_meta_stream_non_lascia_destinazione_ne_residui() {
    let (radice, ingresso, uscita) = ambiente(20_000, Some(15_000));

    let (codice, busta) = converti(&ingresso, &uscita, &[]);

    assert_eq!(codice, Some(3), "il codice d'uscita del formato: {busta}");
    let errore = &busta["error"];
    assert_eq!(errore["code"], "FORMAT_ERROR", "{busta}");
    assert_eq!(errore["category"], "data_mapping", "{busta}");
    assert_eq!(errore["retry"]["kind"], "never", "{busta}");
    assert_eq!(errore["remote_effect"], "none", "{busta}");

    assert!(!uscita.exists(), "la destinazione non deve esistere");
    assert_eq!(
        contenuto(radice.path()),
        vec!["in.csv".to_owned()],
        "nella directory non deve restare uno staging"
    );
}

/// Una destinazione preesistente resta **identica** dopo un tentativo fallito.
///
/// Il rifiuto arriva prima, per no-clobber, e la prova verifica che il file di
/// prima sia ancora quello: un'operazione che fallisce non deve poter toccare
/// cio' che c'era.
#[test]
fn una_destinazione_preesistente_resta_invariata_dopo_un_fallimento() {
    let (radice, ingresso, uscita) = ambiente(2_000, None);
    let prima = b"contenuto preesistente, da non toccare\n";
    std::fs::write(&uscita, prima).expect("la destinazione preesistente si scrive");

    let (codice, busta) = converti(&ingresso, &uscita, &["--max-output-bytes", "1000"]);

    assert_eq!(codice, Some(5), "il codice d'uscita del conflitto: {busta}");
    assert_eq!(busta["error"]["code"], "OUTPUT_EXISTS", "{busta}");
    // Questo rifiuto dichiara `commit`, ed e' corretto: la destinazione si
    // guarda quando la si pubblicherebbe.
    assert_eq!(busta["error"]["phase"], "commit", "{busta}");

    assert_eq!(
        std::fs::read(&uscita).expect("la destinazione si rilegge"),
        prima,
        "il file preesistente non deve essere stato toccato"
    );
    assert_eq!(
        contenuto(radice.path()),
        vec!["in.csv".to_owned(), "uscita.geojson".to_owned()],
        "nella directory non deve restare uno staging"
    );
}
