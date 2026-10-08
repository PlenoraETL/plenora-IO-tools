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

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const fn binario() -> &'static str {
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
            writeln!(testo, "{i},rotto,POINT(non un numero)").expect("la riga si compone");
        } else {
            writeln!(testo, "{i},n{i},POINT({i} {i})").expect("la riga si compone");
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
    // `expect` e non `unwrap_or_default`: col ripiego a stringa vuota
    // l'asserzione qui sotto fallirebbe comunque, ma dicendo «il messaggio non
    // nomina i byte» di un messaggio che non c'e'. Sono due guasti diversi, e
    // conviene che si leggano diversi.
    let messaggio = errore["message"]
        .as_str()
        .expect("la busta d'errore porta un messaggio testuale");
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

/// Un WKT malformato a riga 15.000: l'inferenza lo trova **prima** dello stream.
///
/// # Perche' il nome e' cambiato
///
/// Si chiamava «a meta' stream», e il nome affermava un momento che non
/// avviene. La passata di inferenza di `open` visita la colonna geometrica fino
/// a `max_rows`, quindi incontra la riga rotta mentre allestisce il reader: lo
/// stream non e' cominciato, il writer non esiste. Misurato, non dedotto --
/// marcando il sito dell'inferenza la busta riporta quel marcatore.
///
/// E' percio' la prova del guasto **precoce**, e `phase: prepare` lo dice. Il
/// guasto dentro il loop di lettura ha una prova sua, qui sotto.
#[test]
fn un_wkt_malformato_e_rifiutato_dall_inferenza_prima_dello_stream() {
    let (radice, ingresso, uscita) = ambiente(20_000, Some(15_000));

    let (codice, busta) = converti(&ingresso, &uscita, &[]);

    assert_eq!(codice, Some(3), "il codice d'uscita del formato: {busta}");
    let errore = &busta["error"];
    assert_eq!(errore["code"], "FORMAT_ERROR", "{busta}");
    assert_eq!(errore["category"], "data_mapping", "{busta}");
    assert_eq!(errore["retry"]["kind"], "never", "{busta}");
    assert_eq!(errore["remote_effect"], "none", "{busta}");
    // ERR-003: la fase e' quella in corso, e qui e' l'allestimento del reader.
    // Valeva `validate` finche' la fase la dichiarava il parser, che non sa in
    // quale passata sta girando.
    assert_eq!(
        errore["phase"], "prepare",
        "l'inferenza gira dentro `open`, come `reader_busy`: {busta}"
    );

    assert!(!uscita.exists(), "la destinazione non deve esistere");
    assert_eq!(
        contenuto(radice.path()),
        vec!["in.csv".to_owned()],
        "nella directory non deve restare uno staging"
    );
}

/// Un guasto **dentro il loop di lettura**, e la fase che lo dichiara.
///
/// # Come si arriva qui, visto che l'inferenza vede le stesse celle
///
/// Sfruttando l'unica asimmetria fra le due passate: l'inferenza **analizza**
/// soltanto, il loop analizza e poi **codifica** in WKB. Una LINESTRING scritta
/// con coordinate corte occupa in WKB circa quattro volte il suo testo, quindi
/// esiste un tetto per cella che il WKT passa e il WKB no: 411 byte di testo
/// contro 1613 di WKB, con la soglia a 1000.
///
/// Senza quell'asimmetria questa prova non esisterebbe, e la correzione della
/// fase nel loop sarebbe codice che nessun ingresso raggiunge.
///
/// # Che cosa fissa
///
/// `phase: read`. Il valore prima della correzione era `validate`, verificato
/// rimuovendola e rieseguendo: la busta diceva «non ho cominciato» dopo aver
/// letto e convertito le righe precedenti.
#[test]
fn un_guasto_nel_loop_di_lettura_dichiara_la_fase_di_lettura() {
    let radice = tempfile::tempdir().expect("directory temporanea");
    let ingresso = radice.path().join("in.csv");
    let uscita = radice.path().join("uscita.geojson");

    let vertici: Vec<String> = (0..100).map(|i| format!("{} {}", i % 9, i % 9)).collect();
    let linea = format!("LINESTRING({})", vertici.join(","));
    assert!(
        linea.len() < 1_000 && 13 + 16 * 100 > 1_000,
        "il caso vale solo se il WKT sta sotto il tetto e il WKB no: WKT {} byte",
        linea.len()
    );

    let mut testo = String::from("id,geometry\n");
    for i in 0..50 {
        writeln!(testo, "{i},\"{linea}\"").expect("la riga si compone");
    }
    std::fs::write(&ingresso, testo).expect("la sorgente si scrive");

    let (codice, busta) = converti(&ingresso, &uscita, &["--max-wkb-cell-bytes", "1000"]);

    assert_eq!(codice, Some(3), "il codice d'uscita del formato: {busta}");
    let errore = &busta["error"];
    assert_eq!(
        errore["phase"], "read",
        "il rifiuto arriva nel loop, dopo che `read_record` e' riuscito: {busta}"
    );
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
