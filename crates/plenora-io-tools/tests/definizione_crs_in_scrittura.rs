//! Che cosa ogni bersaglio fa della definizione CRS che riceve, e che lo dica.
//!
//! # Il difetto che queste prove chiudono (rilettura 10)
//!
//! Un Arrow conforme puo' portare un identificatore e una definizione in
//! qualunque grammatica: GEO-005 confronta solo gli identificatori, e un
//! `EPSG:4326` con un `LOCAL_CS` e' un ingresso che si accetta (GEO-000). Tre
//! bersagli dichiaravano `crs_definition: preserved` senza condizioni e non la
//! conservavano:
//!
//! * `GeoParquet` conserva solo il PROJJSON. Con `EPSG:4326` e un WKT ometteva
//!   il campo `crs` -- il file si rilegge `OGC:CRS84` senza definizione -- e la
//!   conversione si dichiarava `lossless`; senza identificatore scriveva
//!   `crs: null`, «CRS ignoto», per un CRS che il contratto conosceva;
//! * Shapefile e DXF conservano solo il WKT. Un PROJJSON finiva nel `.prj` o nel
//!   `GEODATA` tale e quale, e il file non si rileggeva piu'.
//!
//! E un bersaglio che conserva il solo identificatore sceglieva l'identificatore
//! anche quando la definizione lo contraddice nel genere.
//!
//! # Perche' passa dal binario
//!
//! La promessa e' la busta: `write_loss.counts` e `conversion_fidelity`, o il
//! rifiuto con la sua categoria. Ogni prova qui fallisce su `b7a6c94`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const fn binario() -> &'static str {
    env!("CARGO_BIN_EXE_plenora-io")
}

const WGS84_WKT: &str = "GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]";

/// Una radice locale: non e' un CRS orizzontale, e `EPSG:4326` la contraddice.
const LOCAL_CS: &str =
    "LOCAL_CS[\"x\",LOCAL_DATUM[\"d\",0],UNIT[\"metre\",1],AXIS[\"X\",EAST],AXIS[\"Y\",NORTH]]";

/// Un proiettato anonimo: di genere opposto a `EPSG:4326`.
const PROIETTATO_ANONIMO: &str = "PROJCS[\"anon\",GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]],PROJECTION[\"Transverse_Mercator\"],PARAMETER[\"central_meridian\",9],UNIT[\"metre\",1]]";

const PROJJSON_3857: &str = r#"{"type":"ProjectedCRS","name":"WGS 84 / Pseudo-Mercator","base_crs":{"name":"WGS 84","datum":{"type":"GeodeticReferenceFrame","name":"World Geodetic System 1984","ellipsoid":{"name":"WGS 84","semi_major_axis":6378137,"inverse_flattening":298.257223563}},"coordinate_system":{"subtype":"ellipsoidal","axis":[{"name":"Geodetic latitude","abbreviation":"Lat","direction":"north","unit":"degree"},{"name":"Geodetic longitude","abbreviation":"Lon","direction":"east","unit":"degree"}]}},"conversion":{"name":"Popular Visualisation Pseudo-Mercator","method":{"name":"Popular Visualisation Pseudo Mercator"},"parameters":[]},"coordinate_system":{"subtype":"Cartesian","axis":[{"name":"Easting","abbreviation":"X","direction":"east","unit":"metre"},{"name":"Northing","abbreviation":"Y","direction":"north","unit":"metre"}]},"id":{"authority":"EPSG","code":3857}}"#;

const PROJJSON_CRS84: &str = r#"{"type":"GeographicCRS","name":"WGS 84","datum":{"type":"GeodeticReferenceFrame","name":"World Geodetic System 1984","ellipsoid":{"name":"WGS 84","semi_major_axis":6378137,"inverse_flattening":298.257223563}},"coordinate_system":{"subtype":"ellipsoidal","axis":[{"name":"Geodetic longitude","abbreviation":"Lon","direction":"east","unit":"degree"},{"name":"Geodetic latitude","abbreviation":"Lat","direction":"north","unit":"degree"}]},"id":{"authority":"OGC","code":"CRS84"}}"#;

struct Esito {
    riuscita: bool,
    busta: serde_json::Value,
}

impl Esito {
    fn perdite(&self) -> Vec<String> {
        let mut voci: Vec<String> = self.busta["result"]["write_loss"]["counts"]
            .as_array()
            .expect("una scrittura riuscita porta `counts` come elenco")
            .iter()
            .filter_map(|voce| voce["categoria"].as_str())
            .filter(|c| c.starts_with("crs_") || c.starts_with("srid_"))
            .map(str::to_owned)
            .collect();
        voci.sort();
        voci
    }

    fn livello(&self) -> &str {
        self.busta["result"]["conversion_fidelity"]["level"]
            .as_str()
            .expect("la conversione dichiara il livello")
    }

    fn categoria(&self) -> &str {
        self.busta["error"]["category"]
            .as_str()
            .expect("il rifiuto porta la categoria")
    }

    fn messaggio(&self) -> &str {
        self.busta["error"]["message"]
            .as_str()
            .expect("il rifiuto porta il messaggio")
    }
}

fn cli(argomenti: &[&str]) -> Esito {
    let output = Command::new(binario())
        .args(argomenti)
        .output()
        .expect("il binario si esegue");
    Esito {
        riuscita: output.status.success(),
        busta: serde_json::from_str(String::from_utf8_lossy(&output.stdout).trim())
            .expect("un documento JSON su stdout"),
    }
}

fn formato(percorso: &Path) -> &'static str {
    let nome = percorso
        .file_name()
        .and_then(|n| n.to_str())
        .expect("percorso rappresentabile");
    if nome.ends_with(".shp.d") {
        return "shp";
    }
    match percorso.extension().and_then(|e| e.to_str()) {
        Some("parquet") => "geoparquet",
        Some("arrow") => "ipc",
        Some("gpkg") => "gpkg",
        Some("geojson") => "geojson",
        Some("dxf") => "dxf",
        altro => panic!("la prova non sa quale formato chiedere per {altro:?}"),
    }
}

fn converti(ingresso: &Path, uscita: &Path, extra: &[&str]) -> Esito {
    let mut argomenti = vec![
        "convert",
        ingresso.to_str().expect("percorso"),
        uscita.to_str().expect("percorso"),
        "--from",
        formato(ingresso),
        "--to",
        formato(uscita),
    ];
    argomenti.extend_from_slice(extra);
    cli(&argomenti)
}

fn ispeziona(percorso: &Path) -> Esito {
    cli(&["inspect", percorso.to_str().expect("percorso")])
}

/// Un punto in WKB ISO little-endian.
fn punto_wkb(x: f64, y: f64) -> Vec<u8> {
    let mut byte = vec![1_u8];
    byte.extend_from_slice(&1_u32.to_le_bytes());
    byte.extend_from_slice(&x.to_le_bytes());
    byte.extend_from_slice(&y.to_le_bytes());
    byte
}

/// Una sorgente Arrow IPC conforme di una riga: `crs_id` (se c'e'),
/// l'ordine degli assi e la definizione nella grammatica dichiarata.
fn sorgente_ipc(
    dir: &Path,
    nome: &str,
    crs_id: Option<&str>,
    assi: &str,
    definizione: (&str, &str),
) -> PathBuf {
    use arrow_array::{ArrayRef, BinaryArray, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};

    let mut geometria = HashMap::from([
        ("ARROW:extension:name".to_owned(), "geoarrow.wkb".to_owned()),
        ("plenora.field_id".to_owned(), "1".to_owned()),
        ("plenora.geometry.encoding".to_owned(), "wkb".to_owned()),
        ("plenora.geometry.dimensions".to_owned(), "xy".to_owned()),
        (
            "plenora.geometry.spatial_semantics".to_owned(),
            "geometry".to_owned(),
        ),
        (
            "plenora.geometry.precision".to_owned(),
            "float64".to_owned(),
        ),
        (
            "plenora.geometry.types_declaration".to_owned(),
            "exact".to_owned(),
        ),
        ("plenora.geometry.types".to_owned(), "point".to_owned()),
        (
            "plenora.geometry.crs_resolution".to_owned(),
            "resolved".to_owned(),
        ),
        ("plenora.geometry.axis_order".to_owned(), assi.to_owned()),
        (
            "plenora.geometry.crs_definition".to_owned(),
            definizione.0.to_owned(),
        ),
        (
            "plenora.geometry.crs_definition_format".to_owned(),
            definizione.1.to_owned(),
        ),
    ]);
    if let Some(id) = crs_id {
        geometria.insert("plenora.geometry.crs_id".to_owned(), id.to_owned());
        geometria.insert("crs".to_owned(), id.to_owned());
    }
    let schema = Arc::new(Schema::new_with_metadata(
        vec![
            Field::new("nome", DataType::Utf8, false),
            Field::new("geometry", DataType::Binary, true).with_metadata(geometria),
        ],
        HashMap::from([("plenora.contract.version".to_owned(), "1".to_owned())]),
    ));
    let wkb = [Some(punto_wkb(9.19, 45.46))];
    let colonne: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec!["alfa"])),
        Arc::new(wkb.iter().map(Option::as_deref).collect::<BinaryArray>()),
    ];
    let batch = RecordBatch::try_new(Arc::clone(&schema), colonne).expect("batch");
    let percorso = dir.join(nome);
    let file = std::fs::File::create(&percorso).expect("sorgente creata");
    let mut scrittore = arrow_ipc::writer::FileWriter::try_new(file, &schema).expect("writer");
    scrittore.write(&batch).expect("batch scritto");
    scrittore.finish().expect("sorgente completata");
    percorso
}

fn directory() -> tempfile::TempDir {
    tempfile::tempdir().expect("directory temporanea")
}

// --- GeoParquet: conserva solo il PROJJSON --------------------------------

/// `EPSG:4326` con un WKT: il campo `crs` si omette, e la perdita si dichiara.
#[test]
fn geoparquet_dichiara_il_wkt_che_non_scrive() {
    let dir = directory();
    let sorgente = sorgente_ipc(
        dir.path(),
        "wkt.arrow",
        Some("EPSG:4326"),
        "lon_lat",
        (WGS84_WKT, "wkt"),
    );
    let uscita = dir.path().join("uscita.parquet");
    let esito = converti(&sorgente, &uscita, &[]);
    assert!(esito.riuscita, "{}", esito.busta);
    // Dalla rilettura 11 si dichiara anche l'identificatore: il file si
    // rilegge `OGC:CRS84`, e `EPSG:4326` e' un altro identificatore.
    assert_eq!(
        esito.perdite(),
        [
            "crs_definition_not_preserved_absent",
            "crs_id_not_preserved_absent"
        ]
    );
    assert_ne!(
        esito.livello(),
        "lossless",
        "una definizione perduta non e' una conversione lossless"
    );
    assert_eq!(esito.busta["result"]["write_loss"]["lossless"], false);
    // E il file dice cio' che la busta ha dichiarato: CRS84, senza definizione.
    let riletto = ispeziona(&uscita);
    assert!(riletto.riuscita, "{}", riletto.busta);
    let geometria = &riletto.busta["result"]["layers"][0]["geometry"];
    assert_eq!(geometria["crs"], "OGC:CRS84");
    assert!(geometria["crs_resolution"]["definition"].is_null());
}

/// Un WKT senza identificatore: fino a `b7a6c94` usciva `crs: null`.
#[test]
fn geoparquet_rifiuta_un_wkt_senza_identificatore() {
    let dir = directory();
    let sorgente = sorgente_ipc(
        dir.path(),
        "anonimo.arrow",
        None,
        "lon_lat",
        (WGS84_WKT, "wkt"),
    );
    let uscita = dir.path().join("uscita.parquet");
    let esito = converti(&sorgente, &uscita, &[]);
    assert!(!esito.riuscita, "atteso un rifiuto: {}", esito.busta);
    assert_eq!(esito.categoria(), "unsupported");
    assert!(
        esito
            .messaggio()
            .contains("non e' PROJJSON e senza identificatore"),
        "{}",
        esito.messaggio()
    );
    assert!(!uscita.exists(), "nessun file pubblicato");
}

// --- la contraddizione: il bersaglio non sceglie --------------------------

/// `EPSG:4326` su un `LOCAL_CS` e su un `PROJCS` anonimo: l'ingresso e'
/// conforme e IPC lo conserva intero, ma i bersagli che terrebbero il solo
/// identificatore rifiutano invece di sceglierlo.
#[test]
fn l_identificatore_non_si_sceglie_contro_la_definizione() {
    let dir = directory();
    for (nome, definizione) in [
        ("locale.arrow", LOCAL_CS),
        ("proiettato.arrow", PROIETTATO_ANONIMO),
    ] {
        let sorgente = sorgente_ipc(
            dir.path(),
            nome,
            Some("EPSG:4326"),
            "lon_lat",
            (definizione, "wkt"),
        );
        let ipc = dir.path().join(format!("copia-{nome}"));
        let conservata = converti(&sorgente, &ipc, &[]);
        assert!(
            conservata.riuscita && conservata.perdite().is_empty(),
            "{nome}: IPC conserva l'ingresso conforme: {}",
            conservata.busta
        );
        for destinazione in ["uscita.parquet", "uscita.gpkg"] {
            let uscita = dir.path().join(format!("{nome}-{destinazione}"));
            let esito = converti(&sorgente, &uscita, &[]);
            assert!(
                !esito.riuscita,
                "{nome} -> {destinazione}: atteso un rifiuto: {}",
                esito.busta
            );
            assert_eq!(esito.categoria(), "unsupported", "{nome} -> {destinazione}");
            assert!(
                esito.messaggio().contains("si contraddicono nel genere"),
                "{nome} -> {destinazione}: {}",
                esito.messaggio()
            );
        }
    }
    // GeoJSON fissa CRS84: stessa regola sull'identificatore che impone.
    let sorgente = sorgente_ipc(
        dir.path(),
        "locale84.arrow",
        Some("OGC:CRS84"),
        "lon_lat",
        (LOCAL_CS, "wkt"),
    );
    let esito = converti(&sorgente, &dir.path().join("uscita.geojson"), &[]);
    assert!(!esito.riuscita, "atteso un rifiuto: {}", esito.busta);
    assert!(
        esito.messaggio().contains("si contraddicono nel genere"),
        "{}",
        esito.messaggio()
    );
}

// --- Shapefile e DXF: conservano solo il WKT ------------------------------

/// Un PROJJSON con un identificatore di cui non sanno il WKT: rifiuto.
#[test]
fn shp_e_dxf_rifiutano_un_projjson_che_non_sanno_scrivere() {
    let dir = directory();
    let sorgente = sorgente_ipc(
        dir.path(),
        "mercatore.arrow",
        Some("EPSG:3857"),
        "easting_northing",
        (PROJJSON_3857, "projjson"),
    );
    for (destinazione, testo) in [
        ("uscita.shp.d", "il .prj porta un WKT"),
        ("uscita.dxf", "il GEODATA porta un WKT"),
    ] {
        let uscita = dir.path().join(destinazione);
        let esito = converti(&sorgente, &uscita, &[]);
        assert!(!esito.riuscita, "{destinazione}: {}", esito.busta);
        assert_eq!(esito.categoria(), "unsupported", "{destinazione}");
        assert!(esito.messaggio().contains(testo), "{}", esito.messaggio());
        assert!(!uscita.exists(), "{destinazione}: nessun file pubblicato");
    }
}

/// Un PROJJSON CRS84: la definizione si sostituisce con il WKT sintetizzato,
/// la perdita si dichiara, e il file si rilegge.
#[test]
fn shp_e_dxf_sostituiscono_il_projjson_e_lo_dichiarano() {
    let dir = directory();
    let sorgente = sorgente_ipc(
        dir.path(),
        "crs84.arrow",
        Some("OGC:CRS84"),
        "lon_lat",
        (PROJJSON_CRS84, "projjson"),
    );
    for destinazione in ["uscita.shp.d", "uscita.dxf"] {
        let uscita = dir.path().join(destinazione);
        let esito = converti(&sorgente, &uscita, &[]);
        assert!(esito.riuscita, "{destinazione}: {}", esito.busta);
        assert!(
            esito
                .perdite()
                .contains(&"crs_definition_not_preserved_derived".to_owned()),
            "{destinazione}: {:?}",
            esito.perdite()
        );
        let riletto = ispeziona(&uscita);
        assert!(riletto.riuscita, "{destinazione}: {}", riletto.busta);
    }
}

// --- rilettura 11 ---------------------------------------------------------
//
// Ogni prova di questa sezione fallisce su `db1307d`. La regola che le tiene
// insieme e' quella di `rilettura_dei_crs.rs`: il file si rilegge come la
// busta lo dichiara.

/// Una sorgente IPC con il solo identificatore.
fn sorgente_solo_id(dir: &Path, nome: &str, crs_id: &str, assi: &str) -> PathBuf {
    use arrow_array::{ArrayRef, BinaryArray, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    let geometria: HashMap<String, String> = [
        ("ARROW:extension:name", "geoarrow.wkb"),
        ("plenora.field_id", "1"),
        ("plenora.geometry.encoding", "wkb"),
        ("plenora.geometry.dimensions", "xy"),
        ("plenora.geometry.spatial_semantics", "geometry"),
        ("plenora.geometry.precision", "float64"),
        ("plenora.geometry.types_declaration", "exact"),
        ("plenora.geometry.types", "point"),
        ("plenora.geometry.crs_resolution", "resolved"),
        ("plenora.geometry.axis_order", assi),
        ("plenora.geometry.crs_id", crs_id),
        ("crs", crs_id),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let schema = Arc::new(Schema::new_with_metadata(
        vec![
            Field::new("nome", DataType::Utf8, false),
            Field::new("geometry", DataType::Binary, true).with_metadata(geometria),
        ],
        HashMap::from([("plenora.contract.version".to_owned(), "1".to_owned())]),
    ));
    let wkb = [Some(punto_wkb(9.19, 45.46))];
    let colonne: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec!["alfa"])),
        Arc::new(wkb.iter().map(Option::as_deref).collect::<BinaryArray>()),
    ];
    let batch = RecordBatch::try_new(Arc::clone(&schema), colonne).expect("batch");
    let percorso = dir.join(nome);
    let file = std::fs::File::create(&percorso).expect("sorgente creata");
    let mut scrittore = arrow_ipc::writer::FileWriter::try_new(file, &schema).expect("writer");
    scrittore.write(&batch).expect("batch scritto");
    scrittore.finish().expect("sorgente completata");
    percorso
}

fn geometria_riletta(percorso: &Path) -> serde_json::Value {
    let riletto = ispeziona(percorso);
    assert!(riletto.riuscita, "{}", riletto.busta);
    riletto.busta["result"]["layers"][0]["geometry"].clone()
}

const PROJJSON_32632: &str = r#"{"$schema":"https://proj.org/schemas/v0.5/projjson.schema.json","type":"ProjectedCRS","name":"WGS 84 / UTM zone 32N","base_crs":{"name":"WGS 84","datum":{"type":"GeodeticReferenceFrame","name":"World Geodetic System 1984","ellipsoid":{"name":"WGS 84","semi_major_axis":6378137,"inverse_flattening":298.257223563}},"coordinate_system":{"subtype":"ellipsoidal","axis":[{"name":"Geodetic latitude","abbreviation":"Lat","direction":"north","unit":"degree"},{"name":"Geodetic longitude","abbreviation":"Lon","direction":"east","unit":"degree"}]}},"conversion":{"name":"UTM zone 32N","method":{"name":"Transverse Mercator","id":{"authority":"EPSG","code":9807}},"parameters":[{"name":"Latitude of natural origin","value":0,"unit":"degree"},{"name":"Longitude of natural origin","value":9,"unit":"degree"},{"name":"Scale factor at natural origin","value":0.9996,"unit":"unity"},{"name":"False easting","value":500000,"unit":"metre"},{"name":"False northing","value":0,"unit":"metre"}]},"coordinate_system":{"subtype":"Cartesian","axis":[{"name":"Easting","abbreviation":"E","direction":"east","unit":"metre"},{"name":"Northing","abbreviation":"N","direction":"north","unit":"metre"}]},"id":{"authority":"EPSG","code":32632}}"#;

/// F1: `EPSG:4326` senza PROJJSON si rilegge da `GeoParquet` `OGC:CRS84`, e la
/// busta lo dichiara. Su `db1307d` la conversione era `lossless`. Anche da un
/// `GeoPackage`, il caso piu' comune.
#[test]
fn geoparquet_dichiara_l_identificatore_che_diventa_crs84() {
    let dir = directory();
    let sorgente = sorgente_solo_id(dir.path(), "wgs84.arrow", "EPSG:4326", "lon_lat");
    let gpkg = dir.path().join("wgs84.gpkg");
    assert!(converti(&sorgente, &gpkg, &[]).riuscita);
    for (ingresso, nome) in [(&sorgente, "da-ipc.parquet"), (&gpkg, "da-gpkg.parquet")] {
        let uscita = dir.path().join(nome);
        let esito = converti(ingresso, &uscita, &[]);
        assert!(esito.riuscita, "{nome}: {}", esito.busta);
        assert!(
            esito
                .perdite()
                .contains(&"crs_id_not_preserved_absent".to_owned()),
            "{nome}: {:?}",
            esito.perdite()
        );
        assert_ne!(esito.livello(), "lossless", "{nome}");
        assert_eq!(geometria_riletta(&uscita)["crs"], "OGC:CRS84", "{nome}");
    }
    // `OGC:CRS84` resta se stesso: nessuna perdita.
    let crs84 = sorgente_solo_id(dir.path(), "crs84.arrow", "OGC:CRS84", "lon_lat");
    let uscita = dir.path().join("crs84.parquet");
    let esito = converti(&crs84, &uscita, &[]);
    assert!(
        esito.riuscita && esito.perdite().is_empty(),
        "{}",
        esito.busta
    );
    assert_eq!(esito.livello(), "lossless");
}

/// F2: il PROJJSON attraversa `GeoParquet` byte per byte. Su `db1307d` le
/// chiavi uscivano riordinate e i numeri passati da `f64`, con la
/// definizione dichiarata conservata.
#[test]
fn geoparquet_conserva_il_projjson_byte_per_byte() {
    let dir = directory();
    let definizione = PROJJSON_32632
        .replace(
            r#""value":0.9996"#,
            r#""value":0.99960000000000000000000001"#,
        )
        .replace(r#""value":500000"#, r#""value":500000.00000000000000001"#)
        .replace(r#""value":9,"#, r#""value":9E0,"#);
    let sorgente = sorgente_ipc(
        dir.path(),
        "utm.arrow",
        Some("EPSG:32632"),
        "easting_northing",
        (&definizione, "projjson"),
    );
    let uscita = dir.path().join("utm.parquet");
    let esito = converti(&sorgente, &uscita, &[]);
    assert!(esito.riuscita, "{}", esito.busta);
    assert!(esito.perdite().is_empty(), "{:?}", esito.perdite());
    let geometria = geometria_riletta(&uscita);
    assert_eq!(
        geometria["crs_resolution"]["definition"].as_str(),
        Some(definizione.as_str())
    );
    // E di nuovo: GeoParquet -> GeoParquet non cambia un byte.
    let ancora = dir.path().join("ancora.parquet");
    assert!(converti(&uscita, &ancora, &[]).riuscita);
    assert_eq!(
        geometria_riletta(&ancora)["crs_resolution"]["definition"].as_str(),
        Some(definizione.as_str())
    );
}

/// F3: `EPSG:4326` su un PROJJSON `EngineeringCRS` verso `GeoParquet`, e su un
/// `LOCAL_CS` verso Shapefile e DXF: rifiuto, prima di ogni effetto. Su
/// `db1307d` `GeoParquet` scriveva un file che non si rileggeva, e lo Shapefile
/// e il DXF un file che si rileggeva non risolto.
#[test]
fn la_coppia_contraddetta_si_rifiuta_anche_dove_la_definizione_si_conserva() {
    let dir = directory();
    let ingegneria = r#"{"$schema":"https://proj.org/schemas/v0.7/projjson.schema.json","type":"EngineeringCRS","name":"x","datum":{"name":"d"},"coordinate_system":{"subtype":"Cartesian","axis":[{"name":"X","abbreviation":"X","direction":"east","unit":"metre"},{"name":"Y","abbreviation":"Y","direction":"north","unit":"metre"}]},"id":{"authority":"EPSG","code":4326}}"#;
    let projjson = sorgente_ipc(
        dir.path(),
        "ingegneria.arrow",
        Some("EPSG:4326"),
        "lon_lat",
        (ingegneria, "projjson"),
    );
    let locale = sorgente_ipc(
        dir.path(),
        "locale.arrow",
        Some("EPSG:4326"),
        "lon_lat",
        (LOCAL_CS, "wkt"),
    );
    for (sorgente, destinazione) in [
        (&projjson, "uscita.parquet"),
        (&locale, "uscita.shp.d"),
        (&locale, "uscita.dxf"),
    ] {
        let uscita = dir.path().join(destinazione);
        let esito = converti(sorgente, &uscita, &[]);
        assert!(!esito.riuscita, "{destinazione}: {}", esito.busta);
        assert_eq!(esito.categoria(), "unsupported", "{destinazione}");
        assert!(
            esito.messaggio().contains("si contraddicono nel genere"),
            "{destinazione}: {}",
            esito.messaggio()
        );
        assert!(!uscita.exists(), "{destinazione}: nessun file pubblicato");
    }
}

/// F4: un PROJJSON non finisce nella colonna WKT di `gpkg_spatial_ref_sys`.
/// La riga vale `undefined`, la definizione si rilegge assente, e la busta lo
/// dichiara. Su `db1307d` il JSON finiva nella colonna, dichiarato derivato.
#[test]
fn gpkg_non_scrive_un_projjson_nella_colonna_wkt() {
    let dir = directory();
    let sorgente = sorgente_ipc(
        dir.path(),
        "utm.arrow",
        Some("EPSG:32632"),
        "easting_northing",
        (PROJJSON_32632, "projjson"),
    );
    let uscita = dir.path().join("utm.gpkg");
    let esito = converti(&sorgente, &uscita, &[]);
    assert!(esito.riuscita, "{}", esito.busta);
    assert_eq!(esito.perdite(), ["crs_definition_not_preserved_absent"]);
    let geometria = geometria_riletta(&uscita);
    assert_eq!(geometria["crs"], "EPSG:32632");
    assert!(
        geometria["crs_resolution"]["definition"].is_null(),
        "{geometria}"
    );
}

/// F5: un CRS risolto che il file rileggerebbe non risolto si rifiuta, o se
/// ha un identificatore ne dichiara la perdita (`absent`, non `derived`). Su
/// `db1307d` `GeoParquet` era `lossless` e lo Shapefile `conditional` senza
/// ragioni CRS, e il file non si rileggeva.
#[test]
fn un_crs_che_il_file_rilegge_non_risolto_non_si_scrive_in_silenzio() {
    let dir = directory();
    let senza_id = PROJJSON_32632.replace(r#","id":{"authority":"EPSG","code":32632}}"#, "}");
    let projjson = sorgente_ipc(
        dir.path(),
        "pj.arrow",
        None,
        "easting_northing",
        (&senza_id, "projjson"),
    );
    let anonimo = sorgente_ipc(
        dir.path(),
        "anonimo.arrow",
        None,
        "lon_lat",
        (PROIETTATO_ANONIMO, "wkt"),
    );
    for (sorgente, destinazione) in [
        (&projjson, "uscita.parquet"),
        (&anonimo, "uscita.shp.d"),
        (&anonimo, "uscita.dxf"),
    ] {
        let uscita = dir.path().join(destinazione);
        let esito = converti(sorgente, &uscita, &[]);
        assert!(!esito.riuscita, "{destinazione}: {}", esito.busta);
        assert_eq!(esito.categoria(), "unsupported", "{destinazione}");
        assert!(!uscita.exists(), "{destinazione}: nessun file pubblicato");
    }
    // Con un identificatore: si scrive, e la perdita e' `absent`, perche' il
    // `.prj` anonimo non lo restituisce; `--assume-crs` lo riporta.
    let con_id = sorgente_ipc(
        dir.path(),
        "con_id.arrow",
        Some("EPSG:3003"),
        "easting_northing",
        (
            PROIETTATO_ANONIMO.replace("\"anon\"", "\"x\"").as_str(),
            "wkt",
        ),
    );
    let uscita = dir.path().join("con_id.shp.d");
    let esito = converti(&con_id, &uscita, &[]);
    assert!(esito.riuscita, "{}", esito.busta);
    assert_eq!(esito.perdite(), ["crs_id_not_preserved_absent"]);
    let riletto = cli(&[
        "inspect",
        uscita.to_str().expect("percorso"),
        "--assume-crs",
        "EPSG:3003",
    ]);
    assert!(riletto.riuscita, "{}", riletto.busta);
}

/// F6: un `GeoPackage` in WGS 84 scritto da noi si converte in Shapefile e si
/// rilegge `EPSG:4326` (su `db1307d` la riga portava `GEOGCS["WGS 84"]`, e lo
/// Shapefile si rileggeva non risolto); un `GeoJSON` (`OGC:CRS84`) verso
/// `GeoPackage` e' `unsupported`, non `crs`.
#[test]
fn gpkg_porta_wgs84_intero_e_rifiuta_crs84_come_limite() {
    let dir = directory();
    let sorgente = sorgente_solo_id(dir.path(), "wgs84.arrow", "EPSG:4326", "lon_lat");
    let gpkg = dir.path().join("wgs84.gpkg");
    assert!(converti(&sorgente, &gpkg, &[]).riuscita);
    let shp = dir.path().join("wgs84.shp.d");
    let esito = converti(&gpkg, &shp, &[]);
    assert!(esito.riuscita, "{}", esito.busta);
    assert_eq!(geometria_riletta(&shp)["crs"], "EPSG:4326");

    let geojson = dir.path().join("crs84.geojson");
    let crs84 = sorgente_solo_id(dir.path(), "crs84.arrow", "OGC:CRS84", "lon_lat");
    assert!(converti(&crs84, &geojson, &[]).riuscita);
    let uscita = dir.path().join("crs84.gpkg");
    let esito = converti(&geojson, &uscita, &[]);
    assert!(!esito.riuscita, "{}", esito.busta);
    assert_eq!(esito.categoria(), "unsupported");
    assert!(!uscita.exists());

    // Stessa classe: il DXF con il solo identificatore e' un limite del
    // bersaglio, non un CRS sbagliato (su `db1307d` era `crs`).
    let utm = sorgente_solo_id(dir.path(), "utm.arrow", "EPSG:32632", "easting_northing");
    let uscita = dir.path().join("utm.dxf");
    let esito = converti(&utm, &uscita, &[]);
    assert!(!esito.riuscita, "{}", esito.busta);
    assert_eq!(esito.categoria(), "unsupported");
}

/// F7: il catalogo non dice `preserved` per una definizione che dipende
/// dalla grammatica. Su `db1307d` `GeoParquet`, Shapefile e DXF pubblicavano
/// `crs_definition: "preserved"`.
#[test]
fn il_catalogo_dice_condizionata_la_definizione_che_dipende_dalla_grammatica() {
    let catalogo = cli(&["catalog"]);
    assert!(catalogo.riuscita, "{}", catalogo.busta);
    let drivers = catalogo.busta["result"]["drivers"]
        .as_array()
        .expect("drivers");
    let definizione = |id: &str| {
        drivers
            .iter()
            .find(|d| d["id"] == id)
            .map(|d| d["write_capabilities"]["crs_representations"]["crs_definition"].clone())
            .expect("il driver e' nel catalogo")
    };
    for id in ["geoparquet", "shp", "dxf", "gpkg"] {
        assert_eq!(definizione(id), "conditional", "{id}");
    }
    assert_eq!(definizione("ipc"), "preserved");
    assert_eq!(definizione("geojson"), "absent");
}

// --- GeoParquet: un CRS ignoto si scrive `null` ---------------------------

fn fixture(nome: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("canoniche")
        .join(nome)
}

/// Un `missing` si scrive in `GeoParquet` come `crs: null`, e si rilegge
/// `missing`; fino a `b7a6c94` era `unsupported` («richiede un CRS risolto»).
#[test]
fn un_crs_ignoto_si_riscrive_in_geoparquet_come_null() {
    let dir = directory();
    let primo = dir.path().join("ignoto.parquet");
    let esito = converti(&fixture("canonico_senza_crs.arrow"), &primo, &[]);
    assert!(esito.riuscita, "{}", esito.busta);
    assert!(esito.perdite().is_empty(), "{:?}", esito.perdite());

    // GeoParquet -> GeoParquet: il caso che `PRODUCT.md` prometteva.
    let secondo = dir.path().join("ancora.parquet");
    let esito = converti(&primo, &secondo, &[]);
    assert!(esito.riuscita, "{}", esito.busta);
    assert_eq!(esito.livello(), "lossless");
    let riletto = ispeziona(&secondo);
    assert!(riletto.riuscita, "{}", riletto.busta);
    let geometria = &riletto.busta["result"]["layers"][0]["geometry"];
    assert!(geometria["crs"].is_null(), "{geometria}");
    assert_eq!(geometria["crs_resolution"]["status"], "missing");

    // `--assume-crs` su `crs: null`: rifiuto, e il messaggio non dice piu'
    // che GeoParquet l'opzione non la usa.
    let esito = cli(&[
        "inspect",
        secondo.to_str().expect("percorso"),
        "--assume-crs",
        "EPSG:4326",
    ]);
    assert!(!esito.riuscita, "{}", esito.busta);
    assert_eq!(esito.categoria(), "crs");
    assert!(
        esito.messaggio().contains("la sorgente lo dichiara ignoto"),
        "{}",
        esito.messaggio()
    );
}
