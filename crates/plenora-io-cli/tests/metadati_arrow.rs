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
//! ARROW-004 (identità di **ogni** campo, preservata attraverso un giro
//! completo e non ricalcolata),
//! ARROW-005 (geometria WKB canonica con l'estensione `GeoArrow`), ARROW-006
//! (metadati non contraddittori), ARROW-007 (i tre stati del CRS distinti),
//! ARROW-012 (ogni batch conforme allo schema dichiarato).
//!
//! Coperti anche i quattro che la prima stesura aveva lasciato fuori con una
//! ragione invece che con una prova: ARROW-002 (fallire chiuso su una versione
//! più nuova), ARROW-008 (ordine degli assi e formato della definizione
//! sopravvivono al pass-through), ARROW-009 (un consumatore generico non
//! **richiede** metadati del provider), ARROW-010 (chi dichiara lossless
//! preserva i metadati ignoti).
//!
//! Le ragioni non erano sbagliate — «lo esercita un altro modulo», «è una
//! proprietà di ciò che non facciamo» — ma erano argomenti, e un argomento
//! copre un requisito solo finché nessuno lo mette alla prova. Scriverle ha
//! trovato due cose che gli argomenti non avevano visto, e stanno nei commenti
//! delle sonde.
//!
//! Resta fuori **ARROW-011**, e la ragione è dell'antecedente: «An operation
//! advertised with Arrow **stream** output MUST allow the consumer to process
//! batches without first materializing the complete result». Il documento
//! capability di questo artefatto dichiara per `io.read` il solo
//! `application/vnd.apache.arrow.file`, perché CLI-2.0 §4 riserva stdout alla
//! busta e i byte devono andare in un file. L'antecedente è falso su questa
//! superficie, e lo si vede in `capabilities`. Diventa vero con B13, che porta
//! lo streaming, e allora questa sarà la sua sonda.

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

/// ARROW-003 e ARROW-004: **ogni** campo ha un'identità, e sopravvive al giro.
///
/// # Che cosa il requisito chiede, e l'argomento che non bastava
///
/// ARROW-003 è condizionale — «A field *whose identity must survive* rename,
/// projection or **round-trip** MUST carry `plenora.field_id`» — e chi decide
/// quali campi siano in quel caso è il componente.
///
/// La prima stesura di questa sonda fissava che l'identità stesse sulla sola
/// colonna geometrica, e difendeva la scelta così: nessuna superficie pubblica
/// proietta, quindi lo schema consegnato è sempre completo e l'indice
/// posizionale non si sposta. L'argomento copriva la **proiezione** e lasciava
/// fuori la terza parola del requisito.
///
/// Il round trip lo facciamo, ed è una forma di prima classe: `io.read` e
/// `io.write` sono dichiarate l'una l'inversa dell'altra e condividono il
/// contratto d'interscambio. Un consumatore che correlasse dati fra due letture
/// aveva un identificatore solo su cui correlare, e per gli attributi nessuno.
///
/// # Che cosa la sonda misura adesso
///
/// Che ogni campo porti un identificatore, che siano distinti, e che siano gli
/// **stessi** dopo un giro che non cambia niente. La terza è ARROW-004, e non
/// si vede su una lettura sola: preservare è un'affermazione su due momenti.
#[test]
fn arrow_003_004_ogni_campo_ha_un_identita_che_sopravvive_al_giro() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");

    for sorgente in ["canonico.geojson", "canonico.gpkg", "canonico.parquet"] {
        let primo = consegna(
            temporanea.path(),
            &fixture(sorgente),
            &format!("uno-{sorgente}.arrow"),
        );
        let secondo = consegna(temporanea.path(), &primo, &format!("due-{sorgente}.arrow"));

        let identita = |percorso: &Path| -> BTreeMap<String, String> {
            schema_di(percorso)
                .fields()
                .iter()
                .map(|c| {
                    (
                        c.name().clone(),
                        c.metadata()
                            .get("plenora.field_id")
                            .cloned()
                            .unwrap_or_else(|| {
                                panic!(
                                    "{sorgente}: ARROW-003 — «{}» non porta \
                                     `plenora.field_id`",
                                    c.name()
                                )
                            }),
                    )
                })
                .collect()
        };

        let al_primo_giro = identita(&primo);
        let al_secondo = identita(&secondo);

        assert!(
            al_primo_giro.len() > 1,
            "{sorgente}: la fixture ha attributi oltre alla geometria, \
             altrimenti la sonda proverebbe di nuovo la sola geometria"
        );

        let distinti: std::collections::BTreeSet<&String> = al_primo_giro.values().collect();
        assert_eq!(
            distinti.len(),
            al_primo_giro.len(),
            "{sorgente}: due campi con lo stesso identificatore non sono \
             identificati: {al_primo_giro:?}"
        );

        assert_eq!(
            al_primo_giro, al_secondo,
            "{sorgente}: ARROW-004 — gli identificatori dei campi non cambiati \
             devono essere gli stessi dopo un giro che non cambia niente"
        );
    }
}

/// L'identità che arriva dalla sorgente non viene riscritta.
///
/// # Perché questa prova è separata, e perché è quella che conta
///
/// La sonda sopra passerebbe anche su un prodotto che **ricalcola** gli
/// identificatori a ogni scrittura: su un giro che non cambia l'ordine dei
/// campi, ricalcolare e conservare danno lo stesso risultato.
///
/// Sono due comportamenti diversi, e la differenza si vedrà il giorno in cui
/// una proiezione entrerà nel confine pubblico: lì l'indice al momento della
/// scrittura non sarà più quello d'origine, e un prodotto che ricalcola
/// rinominerebbe in silenzio ciò che ARROW-004 chiede di preservare.
///
/// Qui la si costruisce oggi: un file Arrow con identificatori **non**
/// posizionali, riletto e riscritto. Se tornassero `0, 1, 2…` il prodotto
/// starebbe ricalcolando.
#[test]
fn arrow_004_gli_identificatori_della_sorgente_non_si_riscrivono() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let originale = consegna(
        temporanea.path(),
        &fixture("canonico.geojson"),
        "base.arrow",
    );

    // Si riscrive lo stesso file con identificatori spostati di cento: restano
    // distinti e non coincidono con nessun indice.
    let spostato = temporanea.path().join("spostato.arrow");
    {
        let schema = schema_di(&originale);
        let campi: Vec<arrow_schema::Field> = schema
            .fields()
            .iter()
            .enumerate()
            .map(|(indice, c)| {
                let mut metadata = c.metadata().clone();
                metadata.insert("plenora.field_id".to_owned(), (100 + indice).to_string());
                c.as_ref().clone().with_metadata(metadata)
            })
            .collect();
        let nuovo = Schema::new_with_metadata(campi, schema.metadata().clone());

        let sorgente = File::open(&originale).expect("l'originale si apre");
        let lettore = FileReader::try_new(sorgente, None).expect("e' un file IPC");
        let batch: Vec<_> = lettore.map(|b| b.expect("batch leggibile")).collect();

        let destinazione = File::create(&spostato).expect("la destinazione si crea");
        let mut scrittore = arrow_ipc::writer::FileWriter::try_new(destinazione, &nuovo)
            .expect("l'intestazione si scrive");
        for uno in batch {
            let rimappato = arrow_array::RecordBatch::try_new(
                std::sync::Arc::new(nuovo.clone()),
                uno.columns().to_vec(),
            )
            .expect("il batch si rimappa sullo schema nuovo");
            scrittore.write(&rimappato).expect("il batch si scrive");
        }
        scrittore.finish().expect("il file si chiude");
    }

    let riletto = consegna(temporanea.path(), &spostato, "riletto.arrow");
    let identificatori: Vec<String> = schema_di(&riletto)
        .fields()
        .iter()
        .map(|c| {
            c.metadata()
                .get("plenora.field_id")
                .cloned()
                .unwrap_or_default()
        })
        .collect();

    assert!(
        identificatori
            .iter()
            .all(|id| id.parse::<usize>().is_ok_and(|n| n >= 100)),
        "ARROW-004: gli identificatori della sorgente sono stati riscritti con \
         gli indici. Conservarli e ricalcolarli danno lo stesso risultato \
         quando l'ordine non cambia, e sono due comportamenti diversi: \
         {identificatori:?}"
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

/// ARROW-002: una versione di contratto più nuova fallisce chiusa.
///
/// # Perché la prova sta qui e non nel modello
///
/// `plenora-io-model` ha già una sonda su `validate_contract_version`, e prova
/// la funzione. Questa prova il **confine pubblico**: un file Arrow che dichiara
/// `plenora.contract.version: 2` arriva a `io.read` e viene rifiutato con un
/// errore tipizzato invece di essere letto indovinando.
///
/// La differenza non è formale. Una funzione giusta che nessuno chiama sul
/// percorso pubblico è un requisito soddisfatto nel posto sbagliato, e fino a
/// questa sonda nessuno aveva verificato che il percorso ci passasse.
#[test]
fn arrow_002_una_versione_piu_nuova_fallisce_chiusa() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let originale = consegna(temporanea.path(), &fixture("canonico.geojson"), "v1.arrow");
    let futuro = temporanea.path().join("v2.arrow");
    riscrivi_con(&originale, &futuro, |schema| {
        let mut metadata = schema.metadata().clone();
        metadata.insert("plenora.contract.version".to_owned(), "2".to_owned());
        Schema::new_with_metadata(schema.fields().clone(), metadata)
    });

    let esito = Command::new(BINARIO)
        .args(["read", futuro.to_str().unwrap(), "--format", "json"])
        .output()
        .expect("il binario parte");
    let stdout = String::from_utf8(esito.stdout).expect("stdout e' UTF-8");
    let busta: serde_json::Value = serde_json::from_str(&stdout).expect("stdout e' JSON");

    assert_eq!(
        busta["status"], "error",
        "ARROW-002: una versione più nuova non si legge indovinando: {stdout}"
    );
    assert!(
        busta["error"]["category"].is_string(),
        "e il rifiuto è tipizzato"
    );
}

/// ARROW-008: ordine degli assi e formato della definizione sopravvivono.
///
/// # Che cosa l'argomento precedente non copriva
///
/// «Lo esercita `provenienza_crs.rs` lungo tutte le conversioni» era vero e
/// riguardava un'altra cosa: lì si verifica da dove il CRS **arriva**, qui che
/// due chiavi specifiche sopravvivano a un pass-through che si dichiara
/// lossless. Sono due affermazioni, e la seconda non segue dalla prima.
#[test]
fn arrow_008_gli_assi_e_il_formato_della_definizione_sopravvivono() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");

    for sorgente in ["canonico.gpkg", "canonico.parquet"] {
        let primo = consegna(
            temporanea.path(),
            &fixture(sorgente),
            &format!("a-{sorgente}.arrow"),
        );
        let secondo = consegna(temporanea.path(), &primo, &format!("b-{sorgente}.arrow"));

        let all_andata = metadati(geometria(&schema_di(&primo)).metadata())
            .into_iter()
            .filter(|(chiave, _)| {
                matches!(
                    *chiave,
                    "plenora.geometry.axis_order" | "plenora.geometry.crs_definition_format"
                )
            })
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect::<BTreeMap<_, _>>();

        assert!(
            !all_andata.is_empty(),
            "{sorgente}: la fixture deve portarne almeno una, altrimenti la \
             sonda passerebbe per assenza di soggetto"
        );

        let al_ritorno = metadati(geometria(&schema_di(&secondo)).metadata())
            .into_iter()
            .filter(|(chiave, _)| all_andata.contains_key(*chiave))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            all_andata, al_ritorno,
            "{sorgente}: ARROW-008 — sono parte del significato pubblico e \
             devono sopravvivere al pass-through"
        );
    }
}

/// ARROW-009: un consumatore generico non **richiede** metadati del provider.
///
/// # Perché «è una proprietà di ciò che non facciamo» non bastava
///
/// Era un argomento sull'assenza, e l'assenza non si prova guardandosi dentro.
/// Qui si costruisce un file Arrow che porta metadati specifici del provider —
/// nel namespace che `ARROW-INTERCHANGE §5` riserva — e uno che non ne porta
/// affatto, e si verifica che `io.read` legga entrambi allo stesso modo.
#[test]
fn arrow_009_i_metadati_del_provider_non_sono_richiesti() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let senza = consegna(
        temporanea.path(),
        &fixture("canonico.geojson"),
        "senza.arrow",
    );

    let con = temporanea.path().join("con-provider.arrow");
    riscrivi_con(&senza, &con, |schema| {
        let campi: Vec<Field> = schema
            .fields()
            .iter()
            .map(|c| {
                // Solo sulla colonna geometrica. La prima stesura li metteva su
                // ogni campo, e il lettore rispondeva «Arrow IPC contiene più
                // colonne GeoArrow»: una chiave `plenora.geometry.*` su un
                // campo qualunque lo fa contare come geometrico. Il rifiuto è
                // corretto -- due colonne geometriche nel contratto v1 non si
                // possono avere -- ed era la sonda a costruire un file che
                // nessun produttore conforme scriverebbe.
                if !c.metadata().contains_key("ARROW:extension:name") {
                    return c.as_ref().clone();
                }
                let mut metadata = c.metadata().clone();
                metadata.insert(
                    "plenora.geometry.native.postgis.typmod".to_owned(),
                    "geography(PointZM,4326)".to_owned(),
                );
                c.as_ref().clone().with_metadata(metadata)
            })
            .collect();
        Schema::new_with_metadata(campi, schema.metadata().clone())
    });

    let righe = |percorso: &Path| -> u64 {
        let esito = Command::new(BINARIO)
            .args(["read", percorso.to_str().unwrap(), "--format", "json"])
            .output()
            .expect("il binario parte");
        let stdout = String::from_utf8(esito.stdout).expect("stdout e' UTF-8");
        let busta: serde_json::Value = serde_json::from_str(&stdout).expect("stdout e' JSON");
        assert_eq!(busta["status"], "ok", "ARROW-009: {stdout}");
        busta["result"]["rows_read"].as_u64().expect("righe")
    };

    assert_eq!(
        righe(&senza),
        righe(&con),
        "ARROW-009: la presenza o l'assenza di metadati del provider non cambia \
         l'interpretazione del contratto comune"
    );
}

/// ARROW-010: chi dichiara lossless preserva i metadati ignoti.
///
/// # Il requisito, e che cosa lega a che cosa
///
/// «An operation that claims lossless pass-through MUST preserve unknown
/// metadata. An operation that intentionally normalizes or drops metadata MUST
/// report that behavior in its output contract or fidelity result.»
///
/// L'argomento precedente era che `io.read` non dichiara pass-through lossless.
/// È falso quando la sorgente è Arrow: lì la busta rende `fidelity.level:
/// lossless`, e quella **è** la dichiarazione. Il requisito si applica, e questa
/// sonda lo misura: un metadato che nessuno conosce, messo su un file Arrow,
/// deve ritrovarsi dopo il giro — oppure la fedeltà non deve dirsi lossless.
#[test]
fn arrow_010_chi_si_dichiara_lossless_conserva_i_metadati_ignoti() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let base = consegna(
        temporanea.path(),
        &fixture("canonico.geojson"),
        "base.arrow",
    );

    let con_ignoti = temporanea.path().join("con-ignoti.arrow");
    riscrivi_con(&base, &con_ignoti, |schema| {
        let campi: Vec<Field> = schema
            .fields()
            .iter()
            .map(|c| {
                let mut metadata = c.metadata().clone();
                metadata.insert("chiave.che.nessuno.conosce".to_owned(), "valore".to_owned());
                c.as_ref().clone().with_metadata(metadata)
            })
            .collect();
        let mut dello_schema = schema.metadata().clone();
        dello_schema.insert("schema.ignoto".to_owned(), "valore".to_owned());
        Schema::new_with_metadata(campi, dello_schema)
    });

    let uscita = temporanea.path().join("dopo.arrow");
    let esito = Command::new(BINARIO)
        .args([
            "read",
            con_ignoti.to_str().unwrap(),
            "--output",
            uscita.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("il binario parte");
    let stdout = String::from_utf8(esito.stdout).expect("stdout e' UTF-8");
    let busta: serde_json::Value = serde_json::from_str(&stdout).expect("stdout e' JSON");
    assert_eq!(busta["status"], "ok", "{stdout}");

    let dichiarata_lossless = busta["result"]["fidelity"]["level"] == "lossless";
    let schema = schema_di(&uscita);
    let conservato_sul_campo = schema
        .fields()
        .iter()
        .any(|c| c.metadata().contains_key("chiave.che.nessuno.conosce"));
    let conservato_sullo_schema = schema.metadata().contains_key("schema.ignoto");

    assert!(
        !dichiarata_lossless || (conservato_sul_campo && conservato_sullo_schema),
        "ARROW-010: la fedeltà dice `lossless` e i metadati ignoti non ci sono \
         più (campo: {conservato_sul_campo}, schema: {conservato_sullo_schema}). \
         Le due cose non possono stare insieme: o si conservano, o la fedeltà \
         dichiara di averli normalizzati"
    );
}

/// Riscrive un file Arrow cambiandone lo schema, tenendo i batch.
///
/// Serve alle sonde che costruiscono un ingresso che il prodotto non sa
/// produrre: una versione di contratto futura, metadati di un provider, chiavi
/// che nessuno conosce. Costruirli con `arrow-ipc` invece che col nostro writer
/// è il punto -- il nostro writer non li scriverebbe.
fn riscrivi_con(sorgente: &Path, destinazione: &Path, trasforma: impl FnOnce(&Schema) -> Schema) {
    let file = File::open(sorgente).expect("la sorgente si apre");
    let lettore = FileReader::try_new(file, None).expect("e' un file Arrow IPC");
    let schema = lettore.schema();
    let batch: Vec<_> = lettore.map(|b| b.expect("batch leggibile")).collect();

    let nuovo = std::sync::Arc::new(trasforma(schema.as_ref()));
    let uscita = File::create(destinazione).expect("la destinazione si crea");
    let mut scrittore =
        arrow_ipc::writer::FileWriter::try_new(uscita, nuovo.as_ref()).expect("intestazione");
    for uno in batch {
        let rimappato = arrow_array::RecordBatch::try_new(
            std::sync::Arc::clone(&nuovo),
            uno.columns().to_vec(),
        )
        .expect("il batch si rimappa");
        scrittore.write(&rimappato).expect("il batch si scrive");
    }
    scrittore.finish().expect("il file si chiude");
}
