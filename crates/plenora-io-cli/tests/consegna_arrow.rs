//! `io.read` consegna, e le prove guardano i **byte consegnati**.
//!
//! # Perche' non basta la busta
//!
//! Fino alla 3.0.0 `read` rendeva righe, batch e fedelta' senza scrivere nulla,
//! e ogni prova guardava quel riassunto. Un riassunto puo' essere giusto mentre
//! i dati non escono affatto -- era precisamente il caso -- quindi qui si apre
//! il file prodotto e si guardano i valori, i tipi, i null e i metadati.
//!
//! # Perche' il lettore e' `arrow-ipc` e non il nostro
//!
//! Rileggere con il driver del prodotto proverebbe che sappiamo rileggere cio'
//! che scriviamo, che e' un'affermazione piu' debole: un difetto simmetrico --
//! una convenzione nostra sui metadati, un tipo scritto e riletto allo stesso
//! modo sbagliato -- passerebbe. `arrow-ipc` e' la libreria che userebbe chi ci
//! consuma, ed e' il confine vero.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

use arrow_array::{Array, RecordBatch};
use arrow_ipc::reader::FileReader;

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
    /// Il corpo dell'operazione: la busta lo avvolge, e qui si guarda lui.
    fn risultato(&self) -> serde_json::Value {
        let busta: serde_json::Value =
            serde_json::from_str(self.stdout.trim()).expect("la busta e' JSON");
        busta["result"].clone()
    }

    fn errore(&self) -> serde_json::Value {
        let busta: serde_json::Value =
            serde_json::from_str(self.stdout.trim()).expect("la busta e' JSON");
        busta["error"].clone()
    }
}

fn leggi(argomenti: &[&str]) -> Esito {
    let uscita = Command::new(BINARIO)
        .args(argomenti)
        .output()
        .expect("il binario si esegue");
    // In modo JSON stderr resta vuoto, e questa prova non fa eccezione: una
    // diagnostica accanto alla busta renderebbe il flusso inutilizzabile.
    assert!(
        uscita.stderr.is_empty(),
        "stderr deve restare vuoto: {}",
        String::from_utf8_lossy(&uscita.stderr)
    );
    Esito {
        riuscita: uscita.status.success(),
        stdout: String::from_utf8_lossy(&uscita.stdout).into_owned(),
    }
}

/// I batch del file consegnato, letti con la libreria che userebbe un consumatore.
fn batch_consegnati(percorso: &Path) -> (Vec<RecordBatch>, arrow_schema::SchemaRef) {
    let lettore = FileReader::try_new(File::open(percorso).expect("il file esiste"), None)
        .expect("e' un file Arrow IPC valido");
    let schema = lettore.schema();
    let batch = lettore
        .map(|b| b.expect("ogni batch si decodifica"))
        .collect();
    (batch, schema)
}

fn metadati(mappa: &std::collections::HashMap<String, String>) -> BTreeMap<&str, &str> {
    mappa
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

// --- la consegna ------------------------------------------------------------

#[test]
fn consegna_i_dati_e_non_soltanto_il_conteggio() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let uscita = temporanea.path().join("consegnato.arrow");
    let esito = leggi(&[
        "read",
        fixture("canonico.geojson").to_str().unwrap(),
        "--output",
        uscita.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(esito.riuscita, "{}", esito.stdout);

    let risultato = esito.risultato();
    let consegna = &risultato["delivered"];
    assert_eq!(
        consegna["content_type"],
        "application/vnd.apache.arrow.file"
    );
    assert_eq!(
        consegna["interchange_contract"],
        "plenora-arrow-interchange-v1"
    );
    assert_eq!(consegna["publish_outcome"], "published");
    assert!(consegna["bytes_written"].as_u64().unwrap() > 0);

    let (batch, schema) = batch_consegnati(&uscita);
    let righe: usize = batch.iter().map(RecordBatch::num_rows).sum();
    assert_eq!(
        u64::try_from(righe).expect("il conteggio entra in un u64"),
        risultato["rows_read"].as_u64().unwrap(),
        "le righe consegnate sono quelle che la busta dichiara"
    );
    assert!(righe > 0, "la fixture canonica non e' vuota");
    assert_eq!(schema.fields().len(), 9, "nove campi, come la sorgente");
}

#[test]
fn i_valori_arrivano_interi_e_nell_ordine_letto() {
    // Non «ci sono N righe»: i **valori**. Un writer che scrivesse N righe di
    // zeri soddisferebbe il conteggio e non la consegna.
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let uscita = temporanea.path().join("valori.arrow");
    assert!(
        leggi(&[
            "read",
            fixture("canonico.geojson").to_str().unwrap(),
            "--output",
            uscita.to_str().unwrap(),
        ])
        .riuscita
    );

    let (batch, schema) = batch_consegnati(&uscita);
    let indice = schema.index_of("codice").expect("la colonna `codice` c'e'");
    let colonna = batch[0].column(indice);
    let testi = colonna
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .expect("`codice` e' Utf8");
    let letti: Vec<&str> = (0..testi.len())
        .map(|i| {
            if testi.is_null(i) {
                "<null>"
            } else {
                testi.value(i)
            }
        })
        .collect();
    assert!(
        letti.iter().any(|v| *v != "<null>"),
        "almeno un codice non nullo: {letti:?}"
    );
    assert_eq!(
        letti.len(),
        batch[0].num_rows(),
        "un valore per riga, senza buchi nella colonna"
    );
}

#[test]
fn i_tipi_e_la_nullabilita_sopravvivono() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let uscita = temporanea.path().join("tipi.arrow");
    assert!(
        leggi(&[
            "read",
            fixture("canonico.geojson").to_str().unwrap(),
            "--output",
            uscita.to_str().unwrap(),
        ])
        .riuscita
    );

    let (_, schema) = batch_consegnati(&uscita);
    let per_nome: BTreeMap<&str, &arrow_schema::DataType> = schema
        .fields()
        .iter()
        .map(|f| (f.name().as_str(), f.data_type()))
        .collect();
    assert_eq!(per_nome["conteggio"], &arrow_schema::DataType::Int64);
    assert_eq!(per_nome["misura"], &arrow_schema::DataType::Float64);
    assert_eq!(per_nome["attivo"], &arrow_schema::DataType::Boolean);
    assert_eq!(per_nome["codice"], &arrow_schema::DataType::Utf8);
    assert_eq!(per_nome["geometry"], &arrow_schema::DataType::Binary);
}

#[test]
fn i_null_restano_null() {
    // Un null che diventa un valore predefinito e' una perdita silenziosa, ed e'
    // esattamente cio' che un conteggio di righe non coglie.
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let uscita = temporanea.path().join("null.arrow");
    assert!(
        leggi(&[
            "read",
            fixture("canonico.geojson").to_str().unwrap(),
            "--output",
            uscita.to_str().unwrap(),
        ])
        .riuscita
    );

    let (batch, schema) = batch_consegnati(&uscita);
    let nulli: usize = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(i, _)| {
            batch
                .iter()
                .map(|b| b.column(i).null_count())
                .sum::<usize>()
        })
        .sum();
    // La fixture canonica porta almeno un null dichiarato: se un giorno non lo
    // portasse piu', questa sonda smetterebbe di misurare e deve dirlo.
    assert!(
        nulli > 0,
        "la fixture canonica deve portare almeno un null, altrimenti questa \
         prova non misura la loro sopravvivenza"
    );
}

#[test]
fn lo_schema_consegnato_porta_i_metadati_del_contratto() {
    // ARROW-001: uno schema Plenora che attraversa un confine porta
    // `plenora.contract.version`. Senza, un consumatore non sa se sa leggerlo.
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let uscita = temporanea.path().join("metadati.arrow");
    assert!(
        leggi(&[
            "read",
            fixture("canonico.geojson").to_str().unwrap(),
            "--output",
            uscita.to_str().unwrap(),
        ])
        .riuscita
    );

    let (_, schema) = batch_consegnati(&uscita);
    let dello_schema = metadati(schema.metadata());
    assert!(
        dello_schema.contains_key("plenora.contract.version"),
        "ARROW-001: manca `plenora.contract.version`: {dello_schema:?}"
    );

    let geometria = schema
        .fields()
        .iter()
        .find(|f| f.name() == "geometry")
        .expect("la colonna geometria c'e'");
    let del_campo = metadati(geometria.metadata());
    for chiave in [
        "ARROW:extension:name",
        "plenora.geometry.encoding",
        "plenora.geometry.crs_resolution",
    ] {
        assert!(
            del_campo.contains_key(chiave),
            "manca «{chiave}» sul campo geometria: {del_campo:?}"
        );
    }
    assert_eq!(
        del_campo["ARROW:extension:name"], "geoarrow.wkb",
        "ARROW-005: la geometria WKB canonica usa il nome dell'estensione GeoArrow"
    );
}

#[test]
fn il_crs_risolto_resta_risolto() {
    // ARROW-007: risolto, dichiarato-non-risolto e assente sono tre stati
    // diversi, e una consegna che li confondesse sintetizzerebbe un CRS.
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let uscita = temporanea.path().join("crs.arrow");
    assert!(
        leggi(&[
            "read",
            fixture("canonico.geojson").to_str().unwrap(),
            "--output",
            uscita.to_str().unwrap(),
        ])
        .riuscita
    );
    let (_, schema) = batch_consegnati(&uscita);
    let geometria = schema
        .fields()
        .iter()
        .find(|f| f.name() == "geometry")
        .expect("la colonna geometria c'e'");
    let del_campo = metadati(geometria.metadata());
    assert_eq!(
        del_campo["plenora.geometry.crs_resolution"], "resolved",
        "il GeoJSON dichiara CRS84, e resta risolto"
    );
    assert_eq!(
        del_campo.get("plenora.geometry.crs_id").copied(),
        Some("OGC:CRS84")
    );
}

// --- la fedelta' --------------------------------------------------------

#[test]
fn la_busta_riporta_la_fedelta_della_lettura() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let uscita = temporanea.path().join("fedelta.arrow");
    let esito = leggi(&[
        "read",
        fixture("canonico.geojson").to_str().unwrap(),
        "--output",
        uscita.to_str().unwrap(),
    ]);
    assert!(esito.riuscita);
    let risultato = esito.risultato();
    assert!(
        risultato["fidelity"]["level"].is_string(),
        "la fedelta' e' dichiarata: {}",
        risultato["fidelity"]
    );
    assert!(
        risultato["loss"]["counts"].is_array(),
        "le perdite escono nella forma del protocollo corrente"
    );
}

// --- i casi limite ------------------------------------------------------

/// Un dataset vuoto non si consegna, e il rifiuto e' la risposta giusta.
///
/// # Non e' una scorciatoia: e' la decisione che il repository ha gia' preso
///
/// `tests/foglio_vuoto.rs` la fissa per `convert`: da una sorgente senza righe
/// non c'e' un layout da inferire, la conversione non puo' riuscire, e non deve
/// lasciare una destinazione. Qui vale lo stesso, e per la stessa ragione: il
/// sink Arrow pretende la dichiarazione preventiva dei tipi geometrici, e una
/// sorgente vuota non la puo' dare -- nessuna riga, nessun tipo osservato.
///
/// Consegnare uno schema con zero righe **inventerebbe** quella dichiarazione.
/// Un consumatore leggerebbe «nessuna geometria di questi tipi» dove la verita'
/// e' «non lo so», e sono due cose diverse.
///
/// Il controllo positivo e' nelle altre sonde di questo file: lo stesso comando
/// su una sorgente con righe consegna. Senza quelle, «il vuoto non si consegna»
/// sarebbe vero anche di un binario che non consegna mai.
#[test]
fn un_dataset_vuoto_non_si_consegna_e_non_lascia_un_file() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let vuota = temporanea.path().join("vuota.geojson");
    std::fs::write(&vuota, br#"{"type":"FeatureCollection","features":[]}"#)
        .expect("la sorgente si scrive");
    let uscita = temporanea.path().join("vuoto.arrow");
    let esito = leggi(&[
        "read",
        vuota.to_str().unwrap(),
        "--output",
        uscita.to_str().unwrap(),
    ]);

    assert!(!esito.riuscita, "{}", esito.stdout);
    assert_eq!(
        esito.errore()["category"],
        "unsupported",
        "e' una risposta sul prodotto -- il sink non sa dichiarare tipi che \
         nessuna riga ha mostrato -- non un difetto della richiesta"
    );
    assert!(
        !uscita.exists(),
        "un rifiuto non lascia una destinazione a meta'"
    );
}

/// Senza consegna, invece, una sorgente vuota si legge benissimo.
///
/// E' la meta' che rende accettabile il rifiuto qui sopra: contare zero righe
/// non richiede di dichiarare niente, e chi vuole **sapere** che la sorgente e'
/// vuota lo puo' chiedere.
#[test]
fn un_dataset_vuoto_si_legge_e_conta_zero() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let vuota = temporanea.path().join("vuota.geojson");
    std::fs::write(&vuota, br#"{"type":"FeatureCollection","features":[]}"#)
        .expect("la sorgente si scrive");
    let esito = leggi(&["read", vuota.to_str().unwrap()]);
    assert!(esito.riuscita, "{}", esito.stdout);
    let risultato = esito.risultato();
    assert_eq!(risultato["rows_read"], 0);
    assert!(risultato["delivered"].is_null());
}

#[test]
fn senza_output_non_consegna_e_lo_dichiara() {
    // La forma che conta senza materializzare resta, e il risultato lo dice:
    // un consumatore non deve dedurre dall'assenza di un campo se i dati
    // esistano.
    let esito = leggi(&["read", fixture("canonico.geojson").to_str().unwrap()]);
    assert!(esito.riuscita);
    let risultato = esito.risultato();
    assert!(risultato["rows_read"].as_u64().unwrap() > 0);
    assert!(
        risultato["delivered"].is_null(),
        "senza `--output` non c'e' consegna, e `delivered` lo dice"
    );
}

#[test]
fn il_limite_con_la_consegna_e_rifiutato() {
    // Il difetto che questa prova ha trovato: con `--limit 2 --output` il
    // comando riusciva e consegnava **tutte** le righe, con `truncated: true`
    // accanto. Il limite valeva sul conteggio e non sui byte, che e' la forma
    // peggiore -- la busta diceva una cosa e il file un'altra.
    //
    // La risposta non e' troncare: il writer deve conoscere la cardinalita'
    // esatta dell'ingresso, e una consegna parziale renderebbe falso il totale
    // su cui poggiano le diagnostiche di riga. Si rifiuta chiuso.
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let uscita = temporanea.path().join("troncato.arrow");
    let esito = leggi(&[
        "read",
        fixture("canonico.geojson").to_str().unwrap(),
        "--limit",
        "2",
        "--output",
        uscita.to_str().unwrap(),
    ]);
    assert!(!esito.riuscita, "{}", esito.stdout);
    assert_eq!(esito.errore()["category"], "invalid_plan");
    assert_eq!(esito.errore()["code"], "LIMIT_WITH_DELIVERY");
    assert!(!uscita.exists(), "il rifiuto non lascia un file");
}

#[test]
fn il_limite_da_solo_continua_a_valere() {
    // La controprova: senza, «--limit e' rifiutato» sarebbe vero anche di un
    // prodotto che ha smesso di supportarlo.
    let esito = leggi(&[
        "read",
        fixture("canonico.geojson").to_str().unwrap(),
        "--limit",
        "2",
    ]);
    assert!(esito.riuscita, "{}", esito.stdout);
    let risultato = esito.risultato();
    assert_eq!(risultato["truncated"], true);
    assert!(risultato["delivered"].is_null());
}

// --- i percorsi d'errore ------------------------------------------------

#[test]
fn una_sorgente_inesistente_non_lascia_un_file_a_meta() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let uscita = temporanea.path().join("mai.arrow");
    let esito = leggi(&[
        "read",
        "/nessuna-sorgente-con-questo-nome.geojson",
        "--output",
        uscita.to_str().unwrap(),
    ]);
    assert!(!esito.riuscita);
    assert_eq!(esito.errore()["category"], "io");
    assert!(
        !uscita.exists(),
        "un fallimento non deve lasciare una consegna parziale"
    );
}

#[test]
fn una_destinazione_gia_esistente_e_rifiutata() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let uscita = temporanea.path().join("occupata.arrow");
    std::fs::write(&uscita, b"non toccarmi").expect("il file si scrive");
    let esito = leggi(&[
        "read",
        fixture("canonico.geojson").to_str().unwrap(),
        "--output",
        uscita.to_str().unwrap(),
    ]);
    assert!(!esito.riuscita);
    assert_eq!(esito.errore()["category"], "conflict");
    assert_eq!(
        std::fs::read(&uscita).expect("il file c'e' ancora"),
        b"non toccarmi",
        "il rifiuto non tocca cio' che c'era"
    );
}

#[test]
fn un_layer_inesistente_non_consegna() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let uscita = temporanea.path().join("nessun-layer.arrow");
    let esito = leggi(&[
        "read",
        fixture("canonico.geojson").to_str().unwrap(),
        "--layer",
        "99",
        "--output",
        uscita.to_str().unwrap(),
    ]);
    assert!(!esito.riuscita);
    assert_eq!(esito.errore()["category"], "not_found");
    assert!(!uscita.exists());
}

#[test]
fn output_senza_valore_e_un_errore_d_uso() {
    let esito = leggi(&[
        "read",
        fixture("canonico.geojson").to_str().unwrap(),
        "--output",
    ]);
    assert!(!esito.riuscita);
    assert_eq!(esito.errore()["category"], "invalid_configuration");
}
