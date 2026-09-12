//! I requisiti di Arrow Interchange 1.0, verificati **sui byte consegnati**.
//!
//! # Perché non basta che il codice li implementi
//!
//! Il vocabolario è implementato per intero dentro il prodotto, e lo era anche
//! prima che queste prove esistessero. Ciò che mancava è la differenza fra
//! «il codice sa scrivere quei metadati» e «i byte che escono dal confine
//! pubblico li portano»: fino alla 4.0.0 `io.read` non consegnava affatto, e
//! una prova sull'implementazione sarebbe stata verde su un'operazione che non
//! produceva nulla.
//!
//! Qui si apre il file con `arrow-ipc` — la libreria che userebbe chi ci
//! consuma, non il nostro driver — e si verifica un requisito per volta,
//! nominandolo. Un requisito che perdesse copertura deve poter essere nominato
//! anche lui.
//!
//! # Quali requisiti, e quali no
//!
//! Coperti qui: ARROW-001 (versione di contratto nello schema), ARROW-003 e
//! ARROW-004 (identità dei campi preservata attraverso un giro completo),
//! ARROW-005 (geometria WKB canonica con l'estensione `GeoArrow`), ARROW-006
//! (metadati non contraddittori), ARROW-007 (i tre stati del CRS distinti),
//! ARROW-012 (ogni batch conforme allo schema dichiarato).
//!
//! Non coperti, e ciascuno per una ragione: ARROW-002 è una regola sul
//! **consumatore** — fallire chiuso su una versione più nuova — e la si prova
//! dove leggiamo, non dove scriviamo; ARROW-008 riguarda l'ordine degli assi e
//! il formato della definizione, che `provenienza_crs.rs` già esercita lungo
//! tutte le conversioni; ARROW-009 vieta di **richiedere** metadati specifici
//! del provider, ed è una proprietà di ciò che non facciamo; ARROW-010 lega la
//! conservazione dei metadati ignoti a chi dichiara pass-through lossless, e
//! `io.read` non lo dichiara; ARROW-011 governa lo streaming, che è B13.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

use arrow_ipc::reader::FileReader;
use arrow_schema::{Field, Schema};

const BINARIO: &str = env!("CARGO_BIN_EXE_plenora-io");

fn fixture(nome: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("canoniche")
        .join(nome)
}

/// Consegna una fixture e rende il percorso del file Arrow prodotto.
fn consegna(dove: &Path, sorgente: &Path, nome: &str) -> PathBuf {
    let uscita = dove.join(nome);
    let esito = Command::new(BINARIO)
        .args([
            "read",
            sorgente.to_str().unwrap(),
            "--output",
            uscita.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("il binario parte");
    assert!(
        esito.status.success(),
        "la consegna deve riuscire: {}",
        String::from_utf8_lossy(&esito.stdout)
    );
    uscita
}

fn schema_di(percorso: &Path) -> Schema {
    let file = File::open(percorso).expect("il file consegnato si apre");
    let lettore = FileReader::try_new(file, None).expect("e' un file Arrow IPC");
    lettore.schema().as_ref().clone()
}

fn metadati(mappa: &std::collections::HashMap<String, String>) -> BTreeMap<&str, &str> {
    mappa
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

fn geometria(schema: &Schema) -> &Field {
    schema
        .fields()
        .iter()
        .find(|c| c.metadata().contains_key("ARROW:extension:name"))
        .map(std::convert::AsRef::as_ref)
        .expect("lo schema consegnato porta una colonna geometrica")
}

/// ARROW-001: lo schema che attraversa un confine dichiara la versione del
/// contratto.
///
/// Senza, un consumatore non ha modo di sapere se sa leggerlo, e ARROW-002 gli
/// chiede di fallire chiuso su una versione che non supporta — cosa che non può
/// fare se la versione non c'è.
#[test]
fn arrow_001_la_versione_del_contratto_e_nello_schema() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let consegnato = consegna(temporanea.path(), &fixture("canonico.geojson"), "a.arrow");
    let schema = schema_di(&consegnato);
    let dello_schema = metadati(schema.metadata());

    let versione = dello_schema
        .get("plenora.contract.version")
        .expect("ARROW-001: la versione del contratto manca dallo schema");
    assert_eq!(
        *versione, "1",
        "ARROW-001: la versione dichiarata è quella del vocabolario adottato"
    );
}

/// ARROW-003 e ARROW-004: l'identità dichiarata sopravvive a un giro completo.
///
/// # Che cosa il requisito chiede davvero
///
/// ARROW-003 è **condizionale**: «A field *whose identity must survive* rename,
/// projection or round-trip MUST carry `plenora.field_id`». Chi decide quali
/// campi siano in quel caso è il componente, non il contratto. ARROW-004 invece
/// non è condizionale: gli identificatori che ci sono vanno **preservati** per i
/// campi logici non cambiati.
///
/// # Che cosa il prodotto dichiara oggi, e la domanda che lascia aperta
///
/// La misura, non l'implementazione: solo la colonna geometrica porta
/// `plenora.field_id`. Le colonne attributo no.
///
/// Oggi è difendibile, e per una ragione precisa: nessuna superficie pubblica
/// **proietta**. La CLI non ha un argomento per selezionare le colonne, quindi
/// lo schema consegnato è sempre completo e l'indice posizionale — che è ciò su
/// cui `loss.esempi` indicizza con `field_index` — non si sposta mai.
///
/// Smette di esserlo il giorno in cui una proiezione entra nel confine
/// pubblico: allora `field_index` diventerebbe relativo allo schema proiettato,
/// due letture con proiezioni diverse non sarebbero più correlabili, e
/// l'identità degli attributi sarebbe esattamente ciò che «deve sopravvivere».
/// Il piano 4.0.0 lo registra accanto a B13, che è l'intervento che porterebbe
/// quella proiezione.
///
/// Questa sonda verifica quindi ARROW-004 su ciò che l'identità ce l'ha, e
/// fissa **quali** campi la portano: se un giorno un attributo la acquistasse o
/// la geometria la perdesse, il conteggio cambierebbe e lo direbbe.
#[test]
fn arrow_003_004_l_identita_dichiarata_sopravvive_al_giro() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let primo = consegna(temporanea.path(), &fixture("canonico.geojson"), "uno.arrow");
    let secondo = consegna(temporanea.path(), &primo, "due.arrow");

    let identita = |percorso: &Path| -> BTreeMap<String, String> {
        schema_di(percorso)
            .fields()
            .iter()
            .filter_map(|c| {
                c.metadata()
                    .get("plenora.field_id")
                    .map(|id| (c.name().clone(), id.clone()))
            })
            .collect()
    };

    let al_primo_giro = identita(&primo);
    let al_secondo = identita(&secondo);

    assert!(
        !al_primo_giro.is_empty(),
        "ARROW-003: nessun campo dichiara un'identità durevole, e almeno la \
         geometria deve"
    );
    assert_eq!(
        al_primo_giro, al_secondo,
        "ARROW-004: gli identificatori dei campi non cambiati devono essere gli \
         stessi dopo un giro che non cambia niente"
    );

    // Quali campi la portano, oggi: la sola geometria. Non è un'asserzione sul
    // contratto -- che lascia la scelta al componente -- ma sulla scelta, così
    // che cambiarla richieda di dirlo.
    let schema = schema_di(&primo);
    let geometrici: Vec<&str> = schema
        .fields()
        .iter()
        .filter(|c| c.metadata().contains_key("ARROW:extension:name"))
        .map(|c| c.name().as_str())
        .collect();
    assert_eq!(
        al_primo_giro.keys().map(String::as_str).collect::<Vec<_>>(),
        geometrici,
        "oggi l'identità durevole è dichiarata sulla sola colonna geometrica; \
         se questo cambia, cambia una decisione e va scritta"
    );
}

/// ARROW-005: la geometria WKB canonica usa l'estensione `GeoArrow`.
#[test]
fn arrow_005_la_geometria_dichiara_l_estensione_geoarrow() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let consegnato = consegna(temporanea.path(), &fixture("canonico.geojson"), "g.arrow");
    let schema = schema_di(&consegnato);
    let colonna = geometria(&schema);
    let del_campo = metadati(colonna.metadata());

    assert_eq!(
        del_campo["ARROW:extension:name"], "geoarrow.wkb",
        "ARROW-005: la colonna geometrica dichiara l'estensione canonica"
    );
    assert!(
        matches!(
            del_campo.get("plenora.geometry.encoding"),
            Some(&"wkb" | &"ewkb")
        ),
        "ARROW-005: la codifica è una delle due del vocabolario: {del_campo:?}"
    );
}

/// ARROW-006: i metadati geometrici non si contraddicono.
///
/// Le due regole dipendenti di `ARROW-VOCABULARY-1.0 §4` che si osservano su un
/// artefatto consegnato: `types_declaration=exact` esige una lista di tipi non
/// vuota, `unresolved` la vieta. Un artefatto che dichiarasse `exact` senza
/// tipi, o `unresolved` con tipi, direbbe due cose incompatibili — e il
/// requisito vieta al consumatore di sceglierne una.
#[test]
fn arrow_006_la_dichiarazione_dei_tipi_e_coerente_con_la_lista() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    for (sorgente, nome) in [
        ("canonico.geojson", "c1.arrow"),
        ("canonico.gpkg", "c2.arrow"),
        ("canonico.parquet", "c3.arrow"),
    ] {
        let consegnato = consegna(temporanea.path(), &fixture(sorgente), nome);
        let schema = schema_di(&consegnato);
        let del_campo = metadati(geometria(&schema).metadata());

        let dichiarazione = del_campo
            .get("plenora.geometry.types_declaration")
            .copied()
            .unwrap_or_else(|| panic!("{sorgente}: manca `types_declaration`"));
        let tipi = del_campo.get("plenora.geometry.types").copied();

        match dichiarazione {
            "exact" => assert!(
                tipi.is_some_and(|t| !t.is_empty()),
                "{sorgente}: ARROW-006 con §4 — `exact` esige una lista non vuota"
            ),
            "unresolved" => assert!(
                tipi.is_none(),
                "{sorgente}: ARROW-006 con §4 — `unresolved` vieta la lista, e c'è {tipi:?}"
            ),
            "mixed" => {}
            altro => panic!("{sorgente}: `types_declaration` fuori dal vocabolario: {altro}"),
        }
    }
}

/// ARROW-007: risolto, dichiarato-non-risolto e assente restano tre stati.
///
/// # Che cosa questa prova difende
///
/// Non che il CRS sia giusto: che i tre stati non vengano collassati. Un CRS
/// assente presentato come risolto direbbe a chi consuma che può riproiettare;
/// un CRS risolto presentato come assente gli farebbe buttare via la
/// georeferenziazione. Il requisito esiste perché le due confusioni hanno costi
/// opposti e nessuna è visibile nei dati.
///
/// Le regole dipendenti di §4 sono la parte osservabile su un artefatto:
/// `resolved` e `declared_unresolved` esigono un identificatore o una
/// definizione **e** l'ordine degli assi; `missing` li vieta tutti.
#[test]
fn arrow_007_i_tre_stati_del_crs_restano_distinti() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let mut visti = std::collections::BTreeSet::new();

    for (sorgente, nome) in [
        ("canonico.geojson", "r1.arrow"),
        ("canonico.gpkg", "r2.arrow"),
        ("canonico.parquet", "r3.arrow"),
    ] {
        let consegnato = consegna(temporanea.path(), &fixture(sorgente), nome);
        let schema = schema_di(&consegnato);
        let del_campo = metadati(geometria(&schema).metadata());

        let stato = del_campo
            .get("plenora.geometry.crs_resolution")
            .copied()
            .unwrap_or_else(|| panic!("{sorgente}: manca `crs_resolution`"));
        visti.insert(stato.to_owned());

        let identificato = del_campo.contains_key("plenora.geometry.crs_id")
            || del_campo.contains_key("plenora.geometry.crs_definition");
        let assi = del_campo.contains_key("plenora.geometry.axis_order");

        match stato {
            "resolved" | "declared_unresolved" => {
                assert!(
                    identificato,
                    "{sorgente}: §4 — «{stato}» esige un identificatore o una definizione"
                );
                assert!(assi, "{sorgente}: §4 — «{stato}» esige l'ordine degli assi");
            }
            "missing" => {
                assert!(
                    !identificato && !assi,
                    "{sorgente}: §4 — «missing» vieta identificatore, definizione e assi"
                );
            }
            altro => panic!("{sorgente}: stato del CRS fuori dal vocabolario: {altro}"),
        }

        // La definizione e il suo formato viaggiano insieme o non viaggiano.
        assert_eq!(
            del_campo.contains_key("plenora.geometry.crs_definition"),
            del_campo.contains_key("plenora.geometry.crs_definition_format"),
            "{sorgente}: §4 — definizione e formato della definizione o entrambi o nessuno"
        );
    }

    assert!(
        !visti.is_empty(),
        "almeno uno stato osservato fra le tre sorgenti"
    );
}

/// ARROW-012: tutti i batch di uno stream sono conformi allo schema dichiarato.
///
/// Un batch con un altro schema richiederebbe uno stream nuovo o un protocollo
/// versionato dell'operazione, e nessuno dei due è ciò che `io.read` fa. La
/// sonda confronta ogni batch con lo schema dell'intestazione invece di fidarsi
/// che il writer li abbia prodotti dalla stessa fonte.
#[test]
fn arrow_012_ogni_batch_segue_lo_schema_dichiarato() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let consegnato = consegna(temporanea.path(), &fixture("canonico.gpkg"), "b.arrow");

    let file = File::open(&consegnato).expect("il file consegnato si apre");
    let lettore = FileReader::try_new(file, None).expect("e' un file Arrow IPC");
    let dichiarato = lettore.schema();

    let mut batch = 0usize;
    for prodotto in lettore {
        let prodotto = prodotto.expect("il batch si legge");
        assert_eq!(
            prodotto.schema().fields(),
            dichiarato.fields(),
            "ARROW-012: il batch {batch} ha uno schema diverso da quello dichiarato"
        );
        batch += 1;
    }
    assert!(batch > 0, "la fixture produce almeno un batch");
}
