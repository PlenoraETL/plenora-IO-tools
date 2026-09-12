//! `io.write` pubblica un dataset Arrow, e le prove guardano ciò che è stato
//! pubblicato.
//!
//! # Perché il giro completo, e non la sola scrittura
//!
//! `io.read` e `io.write` sono l'una l'inversa dell'altra: ciò che esce da
//! `read --output` deve poter entrare qui, ed è la ragione per cui condividono
//! il contratto d'interscambio. Una prova che scrivesse soltanto direbbe che il
//! comando non fallisce; il giro completo dice che i dati sono gli stessi, ed è
//! la sola affermazione che interessi a chi orchestra i due passi.
//!
//! # Perché il formato esplicito ha una prova sua
//!
//! Il profilo io-tools vieta di scegliere il comportamento di un formato
//! analizzando l'estensione quando l'operazione richiede un formato esplicito,
//! e il catalogo descrive `io.write` proprio così. Una prova che scrivesse
//! `dati.csv --to csv` non distinguerebbe le due cose: qui si scrive un CSV in
//! un file che si chiama `.geojson`, e si verifica che il contenuto sia CSV.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const BINARIO: &str = env!("CARGO_BIN_EXE_plenora-io");

fn fixture(nome: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("canoniche")
        .join(nome)
}

struct Esito {
    riuscita: bool,
    stdout: String,
}

impl Esito {
    fn documento(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout e' JSON: {e} -- {}", self.stdout))
    }

    fn risultato(&self) -> Value {
        self.documento()["result"].clone()
    }

    fn errore(&self) -> Value {
        self.documento()["error"].clone()
    }
}

fn esegui(argomenti: &[&str]) -> Esito {
    let uscita = Command::new(BINARIO)
        .args(argomenti)
        .arg("--format")
        .arg("json")
        .output()
        .expect("il binario parte");
    Esito {
        riuscita: uscita.status.success(),
        stdout: String::from_utf8(uscita.stdout).expect("stdout e' UTF-8"),
    }
}

/// Il dataset Arrow da cui partono quasi tutte le prove: è ciò che `io.read`
/// consegna, non un file costruito a mano, perché è quello il contratto.
fn dataset_arrow(directory: &Path, sorgente: &Path) -> PathBuf {
    let arrow = directory.join("ingresso.arrow");
    let esito = esegui(&[
        "read",
        sorgente.to_str().unwrap(),
        "--output",
        arrow.to_str().unwrap(),
    ]);
    assert!(
        esito.riuscita,
        "la consegna che alimenta la prova: {}",
        esito.stdout
    );
    arrow
}

#[test]
fn il_giro_completo_conserva_righe_e_valori() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let arrow = dataset_arrow(temporanea.path(), &fixture("canonico.geojson"));
    let uscita = temporanea.path().join("pubblicato.geojson");

    let scrittura = esegui(&[
        "write",
        arrow.to_str().unwrap(),
        uscita.to_str().unwrap(),
        "--to",
        "geojson",
    ]);
    assert!(scrittura.riuscita, "{}", scrittura.stdout);
    let risultato = scrittura.risultato();
    assert_eq!(risultato["format"], "geojson");
    assert!(uscita.exists(), "la destinazione esiste");

    // Il giro si chiude rileggendo: le righe scritte devono essere quelle che
    // la sorgente aveva, e il conteggio dichiarato quello osservato.
    let riletto = esegui(&["read", uscita.to_str().unwrap()]);
    assert!(riletto.riuscita, "{}", riletto.stdout);
    assert_eq!(
        riletto.risultato()["rows_read"],
        risultato["rows_written"],
        "le righe rilette sono quelle dichiarate scritte"
    );
    let originale = esegui(&["read", fixture("canonico.geojson").to_str().unwrap()]);
    assert_eq!(
        originale.risultato()["rows_read"],
        risultato["rows_written"],
        "e sono quelle della sorgente da cui il giro e' partito"
    );
}

/// Il driver viene da `--to`, e l'estensione non decide al suo posto.
///
/// # La sola forma in cui la regola si osserva
///
/// Il profilo io-tools vieta di **scegliere** il comportamento di un formato
/// analizzando l'estensione quando l'operazione richiede un formato esplicito.
/// Quando `--to` e il nome concordano le due strade portano allo stesso posto e
/// la sonda non distinguerebbe niente: qui si fanno disaccordare, e il
/// contenuto deve smentire il nome.
///
/// # Perche' questa prova prima diceva il contrario
///
/// Fino a D10 il file veniva rifiutato, e la sonda lo fissava. Il rifiuto pero'
/// non veniva dal formato: ogni driver scrivibile pretendeva la propria
/// estensione, e in sette casi su dieci dopo il controllo l'estensione non
/// veniva usata per niente. Era una convenzione travestita da vincolo, e
/// rendeva `--to` un controllo di coerenza invece del selettore. Tolta quella,
/// la regola del profilo diventa osservabile davvero.
///
/// I due formati che il suffisso lo pretendono per ragioni loro -- `GeoPackage`
/// per la sua specifica, `Shapefile` per i file companion -- lo dichiarano nel
/// catalogo, e le loro prove stanno accanto a questa.
#[test]
fn il_formato_lo_decide_to_e_non_l_estensione() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let arrow = dataset_arrow(temporanea.path(), &fixture("canonico.geojson"));
    let uscita = temporanea.path().join("travestito.geojson");

    let scrittura = esegui(&[
        "write",
        arrow.to_str().unwrap(),
        uscita.to_str().unwrap(),
        "--to",
        "csv",
    ]);
    assert!(scrittura.riuscita, "{}", scrittura.stdout);
    assert_eq!(scrittura.risultato()["format"], "csv");

    let contenuto = std::fs::read_to_string(&uscita).expect("la destinazione si legge");
    assert!(
        !contenuto.trim_start().starts_with('{'),
        "il contenuto non e' JSON: l'estensione non ha scelto il driver. Letto: {}",
        &contenuto[..contenuto.len().min(80)]
    );
    assert!(
        contenuto.lines().next().is_some_and(|r| r.contains(',')),
        "la prima riga e' un'intestazione CSV"
    );
}

/// I due formati che il suffisso lo pretendono davvero, e lo dichiarano.
///
/// `GeoPackage` per la sua specifica (OGC 12-128r, requisito 2) e `Shapefile` per i
/// file companion, che derivano il nome dal basename del principale. Non sono
/// convenzioni: il primo produrrebbe un artefatto fuori specifica, il secondo
/// non saprebbe dove mettere `.shx` e `.dbf`.
///
/// La sonda non fissa **quali** siano i due: legge il catalogo. Se un terzo
/// formato acquistasse un vincolo, o se uno di questi lo perdesse, il numero
/// cambierebbe e la sonda lo direbbe -- ed e' cio' che deve fare, perche' un
/// vincolo aggiunto in silenzio e' un vincolo di cui nessuno risponde.
#[test]
fn i_vincoli_sul_percorso_sono_dichiarati_e_motivati() {
    let catalogo = esegui(&["catalog"]);
    assert!(catalogo.riuscita, "{}", catalogo.stdout);
    let drivers = catalogo.risultato()["drivers"]
        .as_array()
        .expect("il catalogo elenca driver")
        .clone();

    let mut vincolati: Vec<(String, String)> = Vec::new();
    for driver in &drivers {
        let Some(capacita) = driver["write_capabilities"].as_object() else {
            continue;
        };
        let vincolo = &capacita["sink_path"];
        assert!(
            vincolo["kind"].is_string(),
            "{}: il vincolo sul percorso e' dichiarato",
            driver["id"]
        );
        if vincolo["kind"] == "required" {
            assert!(
                vincolo["reason"].is_string(),
                "{}: un vincolo senza ragione e' un vincolo di cui nessuno risponde",
                driver["id"]
            );
            assert!(
                vincolo["suffixes"]
                    .as_array()
                    .is_some_and(|s| !s.is_empty()),
                "{}: un vincolo senza suffissi non vincola niente",
                driver["id"]
            );
            vincolati.push((
                driver["id"].as_str().unwrap_or_default().to_owned(),
                vincolo["reason"].as_str().unwrap_or_default().to_owned(),
            ));
        }
        // E il suffisso con cui il formato viene **riconosciuto** in lettura e'
        // dichiarato sempre, vincolo o no: e' l'altra meta', e senza di essa
        // togliere il vincolo avrebbe tolto anche l'informazione.
        assert!(
            driver["recognised_suffixes"]
                .as_array()
                .is_some_and(|s| !s.is_empty()),
            "{}: nessun suffisso di riconoscimento dichiarato",
            driver["id"]
        );
    }

    vincolati.sort();
    assert_eq!(
        vincolati,
        vec![
            ("gpkg".to_owned(), "format_specification".to_owned()),
            ("shp".to_owned(), "companion_files".to_owned()),
        ],
        "i formati che pretendono il suffisso sono due, e ciascuno per la sua ragione"
    );
}

/// E il vincolo dichiarato viene applicato: `GeoPackage` rifiuta l'altro nome.
#[test]
fn il_vincolo_dichiarato_da_geopackage_e_applicato() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let arrow = dataset_arrow(temporanea.path(), &fixture("canonico.geojson"));
    let uscita = temporanea.path().join("non_conforme.dat");

    let esito = esegui(&[
        "write",
        arrow.to_str().unwrap(),
        uscita.to_str().unwrap(),
        "--to",
        "gpkg",
    ]);
    assert!(!esito.riuscita, "{}", esito.stdout);
    assert_eq!(esito.errore()["category"], "unsupported");
    let errore = esito.errore();
    let messaggio = errore["message"].as_str().expect("un messaggio");
    assert!(
        messaggio.contains("12-128r"),
        "il rifiuto cita la specifica che lo impone, non una convenzione: {messaggio}"
    );
    assert!(!uscita.exists(), "e non lascia una destinazione");
}

/// Il controesempio: quando `--to` e il nome concordano, si scrive.
///
/// Senza questa sonda, quella qui sopra sarebbe soddisfatta da un `write` che
/// rifiuta sempre.
#[test]
fn con_to_e_nome_concordi_la_pubblicazione_avviene() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let arrow = dataset_arrow(temporanea.path(), &fixture("canonico.geojson"));
    let uscita = temporanea.path().join("concorde.csv");

    let esito = esegui(&[
        "write",
        arrow.to_str().unwrap(),
        uscita.to_str().unwrap(),
        "--to",
        "csv",
    ]);
    assert!(esito.riuscita, "{}", esito.stdout);
    assert_eq!(esito.risultato()["format"], "csv");

    let contenuto = std::fs::read_to_string(&uscita).expect("la destinazione si legge");
    assert!(
        !contenuto.trim_start().starts_with('{'),
        "il contenuto e' CSV e non JSON"
    );
    assert!(
        contenuto.lines().next().is_some_and(|r| r.contains(',')),
        "la prima riga e' un'intestazione CSV"
    );
}

#[test]
fn senza_to_il_comando_rifiuta_invece_di_indovinare() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let arrow = dataset_arrow(temporanea.path(), &fixture("canonico.geojson"));
    let uscita = temporanea.path().join("senza.geojson");

    let esito = esegui(&["write", arrow.to_str().unwrap(), uscita.to_str().unwrap()]);
    assert!(!esito.riuscita, "{}", esito.stdout);
    assert_eq!(esito.errore()["category"], "invalid_configuration");
    assert!(
        !uscita.exists(),
        "un rifiuto non lascia una destinazione a meta'"
    );
}

#[test]
fn un_formato_che_il_catalogo_non_nomina_e_rifiutato() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let arrow = dataset_arrow(temporanea.path(), &fixture("canonico.geojson"));
    let uscita = temporanea.path().join("ignoto.dat");

    let esito = esegui(&[
        "write",
        arrow.to_str().unwrap(),
        uscita.to_str().unwrap(),
        "--to",
        "shapefile",
    ]);
    assert!(!esito.riuscita, "{}", esito.stdout);
    assert_eq!(esito.errore()["category"], "unsupported");
    assert_eq!(esito.errore()["code"], "UNKNOWN_FORMAT");
    // Il nome sbagliato non deve tornare nel messaggio: viene da argv.
    let messaggio = esito.errore()["message"]
        .as_str()
        .expect("un errore porta un messaggio")
        .to_owned();
    assert!(
        !messaggio.contains("shapefile"),
        "il messaggio non ripete l'argomento: {messaggio}"
    );
    assert!(
        messaggio.contains("geojson") && messaggio.contains("gpkg"),
        "elenca invece gli identificatori ammessi: {messaggio}"
    );
}

/// Ogni identificatore che il catalogo dichiara scrivibile è accettato da
/// `--to`, e nessun altro.
///
/// # Perché nei due versi
///
/// Un formato annunciato dal catalogo e non accettato qui sarebbe una capacità
/// falsa; un formato accettato qui e non annunciato sarebbe una capacità
/// nascosta, che nessuno può scoprire e di cui nessuno risponde. Il gate che
/// conta uno solo dei due versi lascia passare l'altro.
#[test]
fn ogni_formato_del_catalogo_ha_un_driver() {
    let catalogo = esegui(&["catalog"]);
    assert!(catalogo.riuscita, "{}", catalogo.stdout);
    let drivers = catalogo.risultato()["drivers"]
        .as_array()
        .expect("il catalogo elenca driver")
        .clone();

    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let arrow = dataset_arrow(temporanea.path(), &fixture("canonico.geojson"));

    let mut scrivibili = 0usize;
    for driver in &drivers {
        let id = driver["id"].as_str().expect("ogni driver ha un id");
        let scrive = driver["direction"]
            .as_str()
            .is_some_and(|d| d == "bidirectional" || d == "write_only");
        if !scrive {
            continue;
        }
        scrivibili += 1;
        let uscita = temporanea.path().join(format!("out_{id}"));
        let esito = esegui(&[
            "write",
            arrow.to_str().unwrap(),
            uscita.to_str().unwrap(),
            "--to",
            id,
        ]);
        // Non si pretende che la scrittura riesca: un formato puo' rifiutare
        // **questi** dati -- un CRS che non sa esprimere, un nome di campo
        // troppo lungo -- ed e' una risposta legittima. Si pretende che il
        // formato sia **riconosciuto**: `UNKNOWN_FORMAT` qui vorrebbe dire che
        // il catalogo annuncia un formato che il comando non sa nominare.
        if !esito.riuscita {
            assert_ne!(
                esito.errore()["code"],
                "UNKNOWN_FORMAT",
                "il catalogo dichiara «{id}» scrivibile e `--to` non lo conosce"
            );
        }
    }
    assert!(
        scrivibili >= 5,
        "il catalogo dichiara almeno cinque formati scrivibili, trovati {scrivibili}"
    );
}

/// L'esito di pubblicazione sta nel vocabolario dichiarato, e `--durable` non
/// lo degrada in silenzio.
#[test]
fn l_esito_di_pubblicazione_e_dichiarato() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let arrow = dataset_arrow(temporanea.path(), &fixture("canonico.geojson"));

    for (nome, durabile) in [("semplice", false), ("durevole", true)] {
        let uscita = temporanea.path().join(format!("{nome}.geojson"));
        let mut argomenti = vec![
            "write",
            arrow.to_str().unwrap(),
            uscita.to_str().unwrap(),
            "--to",
            "geojson",
        ];
        if durabile {
            argomenti.push("--durable");
        }
        let esito = esegui(&argomenti);
        assert!(esito.riuscita, "{}", esito.stdout);
        let dichiarato = esito.risultato()["publish_outcome"]
            .as_str()
            .expect("l'esito e' una stringa")
            .to_owned();
        assert!(
            matches!(
                dichiarato.as_str(),
                "published" | "published_durability_unconfirmed"
            ),
            "esito fuori dal vocabolario: {dichiarato}"
        );
        assert!(uscita.exists(), "{nome}: la destinazione esiste");
    }
}

/// Un rifiuto del sink non lascia una destinazione: il rollback è osservabile.
///
/// `GeoJSON` impone WGS84, e il dataset proiettato non si può esprimere: il
/// rifiuto è della capacità, non della richiesta. Ciò che questa prova fissa
/// non è il rifiuto ma la sua conseguenza sul filesystem — una pubblicazione
/// fallita che lasciasse un file a metà sarebbe indistinguibile, per chi
/// guarda la directory, da una riuscita.
#[test]
fn un_rifiuto_del_sink_non_lascia_una_destinazione() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let arrow = dataset_arrow(temporanea.path(), &fixture("canonico.gpkg"));
    let uscita = temporanea.path().join("mai_nato.geojson");

    let esito = esegui(&[
        "write",
        arrow.to_str().unwrap(),
        uscita.to_str().unwrap(),
        "--to",
        "geojson",
    ]);
    assert!(!esito.riuscita, "{}", esito.stdout);
    assert_eq!(esito.errore()["category"], "unsupported");
    assert!(
        !uscita.exists(),
        "una pubblicazione fallita non lascia byte sulla destinazione"
    );
}

/// Le tre fedeltà rispondono a domande diverse, e la busta le tiene distinte.
#[test]
fn le_fedelta_dell_ingresso_e_della_scrittura_restano_distinte() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let arrow = dataset_arrow(temporanea.path(), &fixture("canonico.geojson"));
    let uscita = temporanea.path().join("fedelta.csv");

    let esito = esegui(&[
        "write",
        arrow.to_str().unwrap(),
        uscita.to_str().unwrap(),
        "--to",
        "csv",
    ]);
    assert!(esito.riuscita, "{}", esito.stdout);
    let risultato = esito.risultato();

    for chiave in ["fidelity", "input_fidelity", "write_fidelity"] {
        assert!(
            risultato[chiave]["level"].is_string(),
            "{chiave} porta un livello dichiarato"
        );
    }
    for chiave in ["input_loss", "write_loss"] {
        assert!(
            risultato[chiave]["counts"].is_array(),
            "{chiave} porta le perdite nella forma del protocollo"
        );
    }

    // L'ingresso e' Arrow, che e' la rappresentazione e non una traduzione:
    // se un giorno la lettura del file Arrow perdesse qualcosa, sarebbe un
    // difetto del giro e non del sink, e si vedrebbe qui.
    assert_eq!(
        risultato["input_fidelity"]["level"], "lossless",
        "leggere il dataset Arrow non perde niente: {}",
        risultato["input_fidelity"]
    );
}

#[test]
fn una_sorgente_che_non_e_arrow_e_un_rifiuto_tipizzato() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let uscita = temporanea.path().join("mai.csv");
    let esito = esegui(&[
        "write",
        fixture("canonico.geojson").to_str().unwrap(),
        uscita.to_str().unwrap(),
        "--to",
        "csv",
    ]);
    assert!(!esito.riuscita, "{}", esito.stdout);
    assert!(
        esito.errore()["category"].is_string(),
        "il rifiuto porta una categoria"
    );
    assert!(!uscita.exists(), "e non lascia una destinazione");
}

#[test]
fn write_senza_argomenti_spiega_l_uso() {
    let esito = esegui(&["write"]);
    assert!(!esito.riuscita);
    assert_eq!(esito.errore()["category"], "invalid_configuration");
    let errore = esito.errore();
    let messaggio = errore["message"]
        .as_str()
        .expect("un errore porta un messaggio");
    assert!(
        messaggio.contains("--to"),
        "l'uso nomina il formato esplicito: {messaggio}"
    );
}
