//! Le conversioni cross-format che il prodotto promette, una per una.
//!
//! # Perche' non sono cento
//!
//! Dieci driver R/W non fanno cento conversioni equivalenti: profilo, classe di
//! CRS dell'origine, classe di fedelta' del bersaglio e multi-layer rompono
//! l'equivalenza, e le classi che ne risultano sono molte meno delle coppie.
//! Provare cento coppie con la stessa asserzione costerebbe cento volte e
//! proverebbe una cosa sola.
//!
//! Quali coppie servano non e' scritto qui: lo dichiara
//! `assurance/registries/conversioni-cross-format.json`, e
//! `scripts/check-conversioni.py` **deriva** dal registro se la copertura
//! regge. Il numero dei casi non e' un obiettivo e non e' dichiarato da nessuna
//! parte: se domani ne servisse uno in piu', il gate lo dira'.
//!
//! # Che cosa prova ogni caso
//!
//! Non il successo del comando. Ogni caso riuscito rilegge l'uscita attraverso
//! la busta pubblica -- `read` per i tipi, una normalizzazione in CSV per i
//! valori -- e asserisce il `LossReport` **per intero**: le categorie presenti
//! e quelle assenti. Un caso che passasse con un `LossReport` diverso da quello
//! atteso e' rosso quanto uno che fallisce, perche' cio' che il prodotto
//! promette e' proprio quella dichiarazione.
//!
//! Ogni caso rifiutato nomina il **proprio** rifiuto: codice e messaggio. Un
//! rifiuto contato e non letto passerebbe anche il giorno in cui arriva per una
//! ragione che non c'entra, ed e' successo -- `gpkg -> kml` si fermava su un
//! contratto geometrico incompleto e sembrava il rifiuto sul CRS.

// Il percorso e' esplicito perche' `conversioni.rs` e' la **radice** del
// target di test: per una radice, `mod x;` cerca `tests/x.rs`, non
// `tests/conversioni/x.rs`. Il modulo sta accanto alla suite che serve, ed e'
// il percorso che il registro delle conversioni dichiara.
#[path = "conversioni/canonico.rs"]
mod canonico;

use std::collections::BTreeMap;

use canonico::{attesa, converti, per_id, schema, stessa_geometria, ATTESE};

/// Le opzioni con cui si rilegge un'uscita che il CRS non lo porta.
const CRS_PROIETTATO: &[&str] = &["--assume-crs", "EPSG:3003"];
const WKT: &[&str] = &["--in-opt", "wkt_column=geometry"];

fn temporanea() -> tempfile::TempDir {
    tempfile::tempdir().expect("directory temporanea")
}

/// Le perdite attese, per intero: quelle presenti e nessun'altra.
fn perdite(attese: &[(&str, u64)]) -> BTreeMap<String, u64> {
    attese
        .iter()
        .map(|(categoria, conteggio)| ((*categoria).to_owned(), *conteggio))
        .collect()
}

/// Le opzioni per rileggere un'uscita, concatenate.
fn con<'a>(base: &[&'a str], extra: &[&'a str]) -> Vec<&'a str> {
    base.iter().chain(extra.iter()).copied().collect()
}

// --- i due lossless, nei due versi -----------------------------------------

/// `GeoParquet` e Arrow IPC sono le uniche due classi `Lossless` del catalogo, e
/// fra loro non deve perdersi niente.
#[test]
fn geoparquet_a_ipc_conserva_tutto_e_non_dichiara_perdita() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.arrow");
    let esito = converti("canonico.parquet", &uscita, &[]);
    assert!(esito.riuscito, "{}", esito.stderr);
    assert_eq!(
        esito.perdite(),
        perdite(&[]),
        "una conversione fra due formati lossless non ha niente da dichiarare"
    );
    assert_eq!(esito.righe(), 5);

    let righe = per_id(&uscita, &[]);
    assert_eq!(righe.len(), 5, "le cinque righe arrivano tutte");
    for a in ATTESE {
        let riga = &righe[a.id];
        assert_eq!(
            riga["intero_largo"].as_deref(),
            Some(a.intero_largo),
            "{}: l'intero oltre 2^53 non passa da un double",
            a.id
        );
        assert_eq!(riga["codice"].as_deref(), Some(a.codice), "{}", a.id);
        assert_eq!(riga["istante"].as_deref(), Some(a.istante), "{}", a.id);
        assert_eq!(
            riga["misura"].as_deref(),
            a.misura,
            "{}: il float, compreso il null di r3",
            a.id
        );
        assert!(
            stessa_geometria(riga["geometry"].as_deref(), a.geometria),
            "{}: la geometria, compresa la sua assenza: {:?} contro {:?}",
            a.id,
            riga["geometry"],
            a.geometria
        );
    }
    // I tipi restano quelli: e' la meta' che il CSV non puo' dire.
    let campi = schema(&uscita, &[]);
    let per_nome: BTreeMap<_, _> = campi
        .iter()
        .map(|(nome, tipo, _)| (nome.as_str(), tipo.as_str()))
        .collect();
    assert_eq!(per_nome["intero_largo"], "Int64");
    assert_eq!(per_nome["conteggio"], "Int32");
    assert_eq!(per_nome["attivo"], "Boolean");
    assert_eq!(per_nome["istante"], "Date32");
}

/// Il verso opposto non e' lo stesso esercizio: qui e' `GeoParquet` a dover
/// scrivere il PROJJSON che l'IPC porta.
#[test]
fn ipc_a_geoparquet_conserva_tutto_nel_verso_opposto() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.parquet");
    let esito = converti("canonico.arrow", &uscita, &[]);
    assert!(esito.riuscito, "{}", esito.stderr);
    assert_eq!(
        esito.perdite(),
        perdite(&[]),
        "fino a f95d7b4 dichiarava una perdita di definizione CRS che non avveniva"
    );
    assert!(
        esito.documento()["write_loss"]["lossless"]
            .as_bool()
            .expect("il documento dichiara `lossless`"),
        "una conversione verso un formato lossless che non perde niente non puo' dirsi lossy"
    );
    assert_eq!(per_id(&uscita, &[]).len(), 5);
}

// --- il multi-layer ---------------------------------------------------------

/// L'unica sorgente multi-layer verso un bersaglio che ne accetta uno solo.
#[test]
fn gpkg_multi_layer_a_ipc_dichiara_i_layer_non_scelti() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.arrow");
    let esito = converti("canonico.gpkg", &uscita, &[]);
    assert!(esito.riuscito, "{}", esito.stderr);
    assert_eq!(esito.perdite(), perdite(&[]));
    // Il GeoPackage canonico ha due layer e il bersaglio ne accetta uno: la
    // busta deve dire quale ha convertito, non lasciarlo indovinare.
    let documento = esito.documento();
    let layer = documento["layers"]
        .as_array()
        .expect("la busta elenca i layer convertiti");
    assert_eq!(layer.len(), 1, "un bersaglio single-layer ne converte uno");
    assert_eq!(per_id(&uscita, &[]).len(), 5);
}

// --- la via conforme verso GeoParquet ---------------------------------------

/// `GeoJSON` fissa `OGC:CRS84`, e `GeoParquet` ammette di ometterlo dichiarando
/// proprio quel CRS: e' l'unica via conforme che non chiede un PROJJSON alla
/// sorgente.
#[test]
fn geojson_a_geoparquet_e_il_percorso_conforme_verso_il_parquet() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.parquet");
    let esito = converti("canonico.geojson", &uscita, &[]);
    assert!(esito.riuscito, "{}", esito.stderr);
    assert_eq!(esito.perdite(), perdite(&[]));
    assert_eq!(per_id(&uscita, &[]).len(), 5);
}

// --- fra formati che il CRS lo incorporano ----------------------------------

/// Geometria, tipi e CRS attraverso due formati binari che li portano
/// entrambi.
#[test]
fn shp_a_gpkg_conserva_geometria_crs_e_tipi() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.gpkg");
    let esito = converti("canonico_punti.shp", &uscita, CRS_PROIETTATO);
    assert!(esito.riuscito, "{}", esito.stderr);
    assert_eq!(
        esito.perdite(),
        perdite(&[("crs_definition_not_preserved_derived", 1)]),
        "il GeoPackage riscrive la definizione dal proprio gpkg_spatial_ref_sys \
         invece di conservare il WKT del .prj, e lo dichiara"
    );
    let righe = per_id(&uscita, &[]);
    assert_eq!(righe.len(), 2, "la fixture dei punti porta r1 e r5");
    assert!(
        stessa_geometria(righe["r1"]["geometry"].as_deref(), attesa("r1").geometria),
        "il punto arriva con le proprie coordinate: {:?}",
        righe["r1"]["geometry"]
    );
    assert_eq!(
        righe["r5"]["geometry"], None,
        "e la riga senza geometria resta senza geometria"
    );
}

/// La riga senza geometria attraversa lo `Shapefile`, e i nomi ci stanno.
#[test]
fn geojson_a_shp_conserva_anche_la_riga_senza_geometria() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.shp.d");
    let esito = converti("canonico_punti_dbf.geojson", &uscita, &[]);
    assert!(esito.riuscito, "{}", esito.stderr);
    assert_eq!(
        esito.perdite(),
        perdite(&[("crs_id_not_preserved_derived", 1)]),
        "l'identificatore si rilegge dal .prj che il writer scrive"
    );
    let righe = per_id(&uscita, &[]);
    assert_eq!(
        righe.len(),
        2,
        "entrambe le righe, compresa quella senza geometria"
    );
    assert!(righe["r1"]["geometry"].is_some(), "il punto c'e'");
    assert_eq!(
        righe["r5"]["geometry"], None,
        "e la geometria assente e' arrivata come assente, non come una riga persa"
    );
}

// --- verso i formati che il CRS non lo portano ------------------------------

/// Due formati testuali senza CRS: la conversione riesce e dichiara di non
/// conservarlo.
#[test]
fn xls_a_csv_conserva_null_e_interi_larghi() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.csv");
    let esito = converti("canonico.xlsx", &uscita, &con(CRS_PROIETTATO, WKT));
    assert!(esito.riuscito, "{}", esito.stderr);
    assert_eq!(
        esito.perdite(),
        perdite(&[("crs_id_not_preserved_absent", 1)]),
        "il CSV non porta il CRS, e non c'e' niente da cui ricavarlo"
    );
    let righe = per_id(&uscita, &con(CRS_PROIETTATO, WKT));
    assert_eq!(righe.len(), 5);
    for a in ATTESE {
        assert_eq!(
            righe[a.id]["intero_largo"].as_deref(),
            Some(a.intero_largo),
            "{}: l'XLSX porta l'intero largo come testo, e cosi' resta esatto",
            a.id
        );
    }
    for a in ATTESE {
        assert_eq!(
            righe[a.id]["etichetta"].as_deref(),
            // La stringa vuota di r4 e il null di r2 sono lo stesso campo vuoto
            // in un CSV: e' il formato a non distinguerli, e a dirlo e' il caso
            // che passa da un formato che li distingue.
            a.etichetta.filter(|e| !e.is_empty()),
            "{}: l'etichetta, e la cella vuota che resta vuota",
            a.id
        );
    }
}

/// Da un formato spaziale a uno che spaziale non e'.
#[test]
fn kml_a_xls_dichiara_geometria_e_crs_perduti() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.xlsx");
    let esito = converti("canonico.kml", &uscita, &[]);
    assert!(esito.riuscito, "{}", esito.stderr);
    assert_eq!(
        esito.perdite(),
        perdite(&[("crs_id_not_preserved_absent", 1)]),
        "il CRS e' dichiarato perduto"
    );
    // La geometria non sparisce in silenzio: la fedelta' scende, e con la
    // propria ragione. Le due meta' -- categorie e ragioni -- dicono cose
    // diverse, e nessuna basta da sola.
    let documento = esito.documento();
    assert_eq!(
        documento["conversion_fidelity"]["level"], "approximating",
        "un bersaglio che la geometria non la porta non puo' dirsi lossless"
    );
    let ragioni: Vec<&str> = documento["conversion_fidelity"]["reasons"]
        .as_array()
        .expect("le ragioni sono un elenco")
        .iter()
        .filter_map(|r| r["code"].as_str())
        .collect();
    assert!(
        ragioni.contains(&"format_constraint"),
        "atteso un vincolo di formato fra le ragioni, arrivate {ragioni:?}"
    );
}

/// Tutte e tre le rappresentazioni del CRS si perdono, e tutte e tre sono
/// dichiarate.
#[test]
fn geoparquet_a_csv_dichiara_la_perdita_del_crs() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.csv");
    let esito = converti("canonico.parquet", &uscita, &[]);
    assert!(esito.riuscito, "{}", esito.stderr);
    assert_eq!(
        esito.perdite(),
        perdite(&[
            ("crs_definition_not_preserved_absent", 1),
            ("crs_id_not_preserved_absent", 1),
            ("srid_not_preserved_absent", 1),
        ]),
        "tre rappresentazioni, tre dichiarazioni: nessuna di esse e' derivabile \
         da un CSV, e `absent` e' la categoria che lo dice"
    );
    let righe = per_id(&uscita, &con(CRS_PROIETTATO, WKT));
    assert_eq!(righe.len(), 5);
    assert_eq!(
        righe["r1"]["istante"].as_deref(),
        Some("2026-01-15"),
        "la colonna temporale arriva in ISO: e' cio' che `ExplicitText` promette"
    );
}

// --- fra i due CRS fissi ----------------------------------------------------

/// Due formati che fissano lo **stesso** CRS: il controllo positivo del
/// rifiuto su KML.
#[test]
fn geojson_a_kml_fra_due_crs_fissi_riesce() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.kml");
    let esito = converti("canonico.geojson", &uscita, &[]);
    assert!(esito.riuscito, "{}", esito.stderr);
    let perdite_misurate = esito.perdite();
    assert_eq!(
        perdite_misurate.get("crs_id_not_preserved_derived"),
        Some(&1),
        "l'identificatore si ricava dal CRS fisso del formato"
    );
    assert!(
        perdite_misurate.contains_key("coercion tipo attributo"),
        "KML porta i soli attributi testuali, e la coercizione e' dichiarata: {perdite_misurate:?}"
    );
}

// --- verso e da l'unico formato approssimante -------------------------------

/// Le geometrie arrivano, gli attributi no, e ciascuno e' dichiarato per nome.
#[test]
fn geoparquet_a_dxf_dichiara_ogni_approssimazione() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.dxf");
    let esito = converti("canonico_pieno.parquet", &uscita, &[]);
    assert!(esito.riuscito, "{}", esito.stderr);
    let perdite_misurate = esito.perdite();
    for colonna in [
        "id",
        "codice",
        "etichetta",
        "intero_largo",
        "conteggio",
        "misura",
        "attivo",
        "istante",
    ] {
        let categoria = format!("attributo non rappresentato in DXF: {colonna}");
        assert!(
            perdite_misurate.contains_key(&categoria),
            "ogni attributo perduto va nominato, manca «{colonna}»: {perdite_misurate:?}"
        );
    }
    assert_eq!(
        esito.righe(),
        4,
        "la fixture piena porta le quattro righe con geometria"
    );
}

/// Il verso di ritorno: cio' che esce e' cio' che il DXF aveva conservato.
#[test]
fn dxf_a_geojson_restituisce_solo_cio_che_il_dxf_aveva_conservato() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.geojson");
    let esito = converti(
        "canonico_geografico.dxf",
        &uscita,
        &["--assume-crs", "OGC:CRS84"],
    );
    assert!(esito.riuscito, "{}", esito.stderr);
    assert_eq!(
        esito.perdite(),
        perdite(&[("crs_id_not_preserved_derived", 1)]),
        "GeoJSON fissa il proprio CRS, e l'identificatore si ricava da li'"
    );
    assert_eq!(
        esito.righe(),
        4,
        "le quattro entita' del disegno, e nessun attributo da perdere"
    );
}

// --- il percorso che passa da GDAL ------------------------------------------

/// I valori attraversano il confine FFI e tornano identici.
#[cfg(feature = "gdal-backend")]
#[test]
fn filegdb_a_gpkg_attraversa_gdal_senza_alterare_i_valori() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.gpkg");
    let opzioni = con(CRS_PROIETTATO, &["--layer", "0"]);
    let esito = converti("canonico.gdb", &uscita, &opzioni);
    assert!(esito.riuscito, "{}", esito.stderr);
    assert_eq!(
        esito.perdite(),
        perdite(&[("crs_definition_not_preserved_derived", 1)]),
        "la definizione viene riscritta dal GeoPackage, e lo dichiara"
    );
    let righe = per_id(&uscita, &[]);
    assert!(
        !righe.is_empty(),
        "la feature class scelta porta le proprie righe"
    );
    for (id, riga) in &righe {
        assert_eq!(
            riga["codice"].as_deref(),
            Some(attesa(id).codice),
            "{id}: il valore attraversa GDAL senza essere alterato"
        );
    }
}

/// Una tabella non spaziale diventa un CSV di attributi, e nient'altro.
#[cfg(feature = "gdal-backend")]
#[test]
fn filegdb_tabella_non_spaziale_diventa_un_csv_di_attributi() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.csv");
    let opzioni = con(CRS_PROIETTATO, &["--layer", "4"]);
    let esito = converti("canonico.gdb", &uscita, &opzioni);
    assert!(esito.riuscito, "{}", esito.stderr);
    assert_eq!(
        esito.perdite(),
        perdite(&[]),
        "una tabella senza geometria non ha un CRS da perdere"
    );
    let testo = std::fs::read_to_string(&uscita).expect("il CSV si legge");
    let intestazione = testo.lines().next().expect("il CSV ha un'intestazione");
    assert!(
        !intestazione.contains("geometry"),
        "nessuna colonna geometrica inventata: «{intestazione}»"
    );
}

/// Il verso di scrittura verso GDAL, coi tipi che il formato rappresenta.
#[cfg(feature = "gdal-backend")]
#[test]
fn csv_a_filegdb_con_i_tipi_che_il_formato_rappresenta() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.gdb");
    let esito = converti("canonico_filegdb.csv", &uscita, &con(CRS_PROIETTATO, WKT));
    assert!(esito.riuscito, "{}", esito.stderr);
    assert_eq!(
        esito.perdite(),
        perdite(&[("crs_id_not_preserved_derived", 1)]),
        "il CRS lo ricava GDAL a runtime"
    );
    assert_eq!(esito.righe(), 2, "la fixture ristretta porta r1 e r5");
}

// --- i rifiuti, ciascuno con la propria ragione -----------------------------

/// `GeoParquet` pretende un PROJJSON, e un WKT non lo e'.
#[test]
fn gpkg_con_definizione_wkt_a_geoparquet_rifiuta_il_crs() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.parquet");
    let esito = converti("canonico.gpkg", &uscita, &[]);
    assert!(!esito.riuscito, "atteso un rifiuto: {}", esito.stdout);
    assert!(
        esito.messaggio().contains("PROJJSON"),
        "atteso il rifiuto sul documento CRS, arrivato «{}»",
        esito.messaggio()
    );
    assert!(
        !uscita.exists(),
        "un rifiuto non lascia una destinazione pubblicata"
    );
}

/// Lo stesso rifiuto dall'altro estremo: il solo identificatore.
#[test]
fn csv_con_solo_identificatore_a_geoparquet_rifiuta_il_crs() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.parquet");
    let esito = converti("canonico.csv", &uscita, &con(CRS_PROIETTATO, WKT));
    assert!(!esito.riuscito, "atteso un rifiuto: {}", esito.stdout);
    assert!(
        esito.messaggio().contains("solo per identificatore"),
        "atteso il rifiuto sull'identificatore, arrivato «{}»",
        esito.messaggio()
    );
    assert!(!uscita.exists());
}

/// Un nome che il DBF non porta: rifiuto, non troncamento silenzioso.
#[test]
fn geojson_con_nome_lungo_a_shp_rifiuta_invece_di_troncare() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.shp.d");
    let esito = converti("canonico_punti.geojson", &uscita, &[]);
    assert!(!esito.riuscito, "atteso un rifiuto: {}", esito.stdout);
    assert!(
        esito.messaggio().contains("nome oltre il limite"),
        "atteso il rifiuto sul nome, arrivato «{}»",
        esito.messaggio()
    );
}

/// Una riga senza geometria verso un formato di disegno.
#[test]
fn geoparquet_con_riga_senza_geometria_a_dxf_rifiuta_la_riga() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.dxf");
    let esito = converti("canonico.parquet", &uscita, &[]);
    assert!(!esito.riuscito, "atteso un rifiuto: {}", esito.stdout);
    let errore = esito.errore();
    let diagnostica = &errore["row_diagnostics"];
    assert_eq!(
        diagnostica["counts"]["dxf.null_geometry_unsupported"], 1,
        "il rifiuto e' di riga e nomina la propria causa: {errore}"
    );
    let esempio = &diagnostica["examples"][0];
    assert_eq!(
        esempio["source_index"], 4,
        "e dice **quale** riga, con l'indice della sorgente"
    );
}

/// Il booleano, che `FileGDB` non round-trippa.
#[cfg(feature = "gdal-backend")]
#[test]
fn csv_con_booleano_a_filegdb_rifiuta_il_tipo() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.gdb");
    let esito = converti("canonico.csv", &uscita, &con(CRS_PROIETTATO, WKT));
    assert!(!esito.riuscito, "atteso un rifiuto: {}", esito.stdout);
    assert!(
        esito.messaggio().contains("boolean"),
        "atteso il rifiuto sul tipo, arrivato «{}»",
        esito.messaggio()
    );
}

/// Un CRS proiettato verso un formato che ne impone uno fisso.
#[test]
fn gpkg_proiettato_a_kml_rifiuta_il_crs() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.kml");
    let esito = converti("canonico.gpkg", &uscita, &[]);
    assert!(!esito.riuscito, "atteso un rifiuto: {}", esito.stdout);
    assert!(
        esito.messaggio().contains("CRS fisso"),
        "il rifiuto dev'essere quello sul CRS, non uno che arriva prima: «{}»",
        esito.messaggio()
    );
}

/// La tabella verso un bersaglio che una geometria la pretende.
#[cfg(feature = "gdal-backend")]
#[test]
fn tabella_non_spaziale_a_geoparquet_rifiuta_per_capability_del_bersaglio() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.parquet");
    let opzioni = con(CRS_PROIETTATO, &["--layer", "4"]);
    let esito = converti("canonico.gdb", &uscita, &opzioni);
    assert!(!esito.riuscito, "atteso un rifiuto: {}", esito.stdout);
    let errore = esito.errore();
    assert_eq!(
        errore["phase"], "validate",
        "e' una capability del bersaglio, non un errore scoperto leggendo: {errore}"
    );
    assert!(
        esito
            .messaggio()
            .contains("richiede una colonna geometrica"),
        "arrivato «{}»",
        esito.messaggio()
    );
}

/// Lo stesso comando che riesce nel profilo `filegdb` fallisce in quello base,
/// e con la ragione giusta.
#[cfg(not(feature = "gdal-backend"))]
#[test]
fn filegdb_su_profilo_base_rifiuta_con_la_ragione_giusta() {
    let dir = temporanea();
    let uscita = dir.path().join("uscita.gpkg");
    let opzioni = con(CRS_PROIETTATO, &["--layer", "0"]);
    let esito = converti("canonico.gdb", &uscita, &opzioni);
    assert!(!esito.riuscito, "atteso un rifiuto: {}", esito.stdout);
    assert!(
        esito.messaggio().contains("gdal-backend"),
        "il rifiuto deve nominare la feature mancante, non fermarsi altrove: «{}»",
        esito.messaggio()
    );
}
