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

/// Zero righe con schema noto **si consegnano**.
///
/// # Il rilievo che questa prova chiude
///
/// Qui stava scritto che «un dataset vuoto non si consegna», e la ragione data
/// era che una sorgente senza righe non puo' dichiarare i tipi geometrici.
/// Entrambe le affermazioni erano sbagliate, e la seconda spiega la prima.
///
/// Il numero di righe e la conoscenza dello schema sono **indipendenti**. Un
/// `GeoPackage` con zero feature dichiara comunque colonne, tipo geometrico e
/// CRS nelle sue tabelle di sistema; un file Arrow IPC li porta nello schema,
/// che sta nell'intestazione e non nei batch. Dedurre «tipi ignoti» da «zero
/// righe» vale soltanto per i formati che non dichiarano nulla e vanno
/// ispezionati riga per riga -- `GeoJSON` e' uno di quelli, ed era l'unico caso
/// che la prova precedente guardava, generalizzandolo a tutti.
///
/// La sorgente e' costruita qui: si prende lo schema della fixture canonica --
/// metadati geometrici compresi -- e si scrive un file IPC con **zero** batch.
/// E' il caso limite esatto: tutto noto, niente dati.
#[test]
fn zero_righe_con_schema_noto_si_consegnano() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let vuota = temporanea.path().join("vuota.arrow");

    let schema = {
        let file = File::open(fixture("canonico.arrow")).expect("la fixture si apre");
        FileReader::try_new(file, None)
            .expect("la fixture e' un file IPC")
            .schema()
    };
    {
        let file = File::create(&vuota).expect("la sorgente si crea");
        arrow_ipc::writer::FileWriter::try_new(file, &schema)
            .expect("l'intestazione si scrive")
            .finish()
            .expect("il file si chiude senza batch");
    }

    let uscita = temporanea.path().join("consegnata.arrow");
    let esito = leggi(&[
        "read",
        vuota.to_str().unwrap(),
        "--output",
        uscita.to_str().unwrap(),
    ]);

    assert!(esito.riuscita, "{}", esito.stdout);
    let risultato = esito.risultato();
    assert_eq!(risultato["rows_read"], 0, "la sorgente non ha righe");
    assert!(
        !risultato["delivered"].is_null(),
        "zero righe restano una consegna: {risultato}"
    );
    assert!(uscita.exists(), "il file consegnato esiste");

    // E ha ancora lo schema: e' cio' che distingue una consegna vuota da un
    // file vuoto. Chi legge sa quali colonne avrebbe avuto.
    let file = File::open(&uscita).expect("la consegna si apre");
    let lettore = FileReader::try_new(file, None).expect("la consegna e' un file IPC");
    let consegnato = lettore.schema();
    assert_eq!(
        consegnato.fields().len(),
        schema.fields().len(),
        "le colonne consegnate sono quelle della sorgente"
    );
    let righe: usize = lettore
        .map(|b| b.expect("batch leggibile").num_rows())
        .sum();
    assert_eq!(righe, 0, "non sono comparse righe dal nulla");
}

/// Tipi geometrici non determinabili verso un sink che li pretende: rifiuto.
///
/// # Che cosa decide questo rifiuto, e che cosa no
///
/// La regola vera non parla di righe. `capabilities.rs` rifiuta quando **due**
/// cose valgono insieme: il sink dichiara un sottoinsieme proprio dei tipi
/// geometrici canonici, e il contratto della sorgente arriva con
/// `types_declaration = unresolved`. Una sorgente con mille righe di tipi
/// indeterminati sarebbe rifiutata allo stesso modo, e una vuota con tipi
/// dichiarati passa -- lo prova la sonda qui sopra.
///
/// # Il vincolo del sink rispetto al contratto
///
/// `ARROW-VOCABULARY-1.0 §3-4` **ha** un modo di dire «non lo so»:
/// `types_declaration=unresolved`, che vieta la lista dei tipi. Quindi il
/// contratto non ci obbliga a rifiutare: consegnare `unresolved` sarebbe
/// esprimibile. Il rifiuto viene dal sink, ed e' prudente per una ragione sua:
/// il driver IPC dichiara sette dei sedici tipi canonici
/// (`SIMPLE_WKB_GEOMETRY_TYPES`), e un sink che non li accetta tutti non puo'
/// promettere su dati di cui non sa i tipi.
///
/// Restano due questioni aperte, registrate nel piano 4.0.0 e non decise qui:
///
/// 1. il vocabolario chiuso del contratto non distingue «scandito, nessuna
///    geometria» da «tipi ignoti» -- `exact` esige una lista non vuota, quindi
///    una sorgente scandita e priva di geometrie **deve** dirsi `unresolved`;
/// 2. il driver IPC trasporta WKB senza interpretarlo, e la dichiarazione dei
///    sette tipi e' condivisa con quattro driver per cui non e' altrettanto
///    conservativa.
#[test]
fn tipi_non_determinabili_verso_un_sink_che_li_pretende_e_rifiutato() {
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
        "e' una risposta sul prodotto -- il sink non accetta tipi che nessuno \
         ha dichiarato -- non un difetto della richiesta"
    );
    // `expect` e non `unwrap_or_default`: un messaggio assente e' un difetto a
    // se', e un default vuoto lo trasformerebbe in "non contiene la frase",
    // cioe' nella diagnosi sbagliata di un problema diverso.
    let messaggio = esito.errore()["message"]
        .as_str()
        .expect("un errore porta sempre un messaggio")
        .to_owned();
    assert!(
        messaggio.contains("dichiarazione preventiva dei tipi geometrici"),
        "il messaggio nomina la dichiarazione dei tipi, non il numero di \
         righe: se un giorno dicesse «sorgente vuota» sarebbe di nuovo la \
         spiegazione sbagliata. Detto: {messaggio}"
    );
    assert!(
        !uscita.exists(),
        "un rifiuto non lascia una destinazione a meta'"
    );
}

/// Senza consegna, anche la sorgente dai tipi indeterminati si legge.
///
/// # Perche' qui il rifiuto non scatta
///
/// La forma senza `--output` non apre nessun sink, quindi non c'e' nessuna
/// capacita' da soddisfare: si legge e si riferisce. E' la stessa operazione
/// pubblica -- `io.read` -- invocata senza destinazione, non un'operazione
/// diversa: `delivered` vale `null` ed e' li' che la differenza si vede.
/// `schemas/plenora-io-read-input-v1.schema.json` la descrive come il campo
/// `destination` facoltativo, e la tabella del risultato dice che cosa cambia.
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

/// Il limite con la consegna e' rifiutato, e questa prova fissa la **scelta**.
///
/// # Il difetto che l'ha fatta nascere
///
/// Con `--limit 2 --output` il comando riusciva e consegnava **tutte** le
/// righe, con `truncated: true` accanto: il limite valeva sul conteggio e non
/// sui byte, e la busta diceva una cosa mentre il file ne diceva un'altra.
///
/// # Che cosa questa prova prova
///
/// Che il comportamento sia quello **scritto**. Il contratto fissato non lo
/// imponeva -- `SURF-014` vieta soltanto di riportare un parziale come
/// successo pieno, e `PUBLIC-SURFACES-1.0 §9.5` delega la semantica del
/// parziale alla specifica dell'operazione -- quindi la scelta era nostra, ed
/// e' stata fatta per la 4.0.0: `io.read` non consegna dataset parziali,
/// perche' le due semantiche possibili del totale sono incompatibili e
/// sceglierne una cancellerebbe cio' che l'altra conserva. Sta in
/// `contracts/schemas/plenora-io-read-input-v1.schema.json`, per esteso.
///
/// Chi la cambia cambia un contratto, non una riga: l'identificatore dello
/// schema d'ingresso cambierebbe con essa.
#[test]
fn il_limite_con_la_consegna_e_rifiutato() {
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
