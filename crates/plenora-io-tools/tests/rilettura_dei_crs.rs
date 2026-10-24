//! La matrice delle riletture: ogni sorgente verso ogni bersaglio, e il file
//! scritto si rilegge.
//!
//! # L'invariante
//!
//! Una scrittura che riesce dice, in `write_loss.counts`, che cosa del CRS non
//! ha conservato. Il file che produce deve dire **la stessa cosa**:
//!
//! 1. senza una perdita `crs_id_not_preserved_*`, il file si rilegge con lo
//!    **stesso** identificatore, byte per byte;
//! 2. con `crs_id_not_preserved_derived`, l'identificatore si rilegge dal file
//!    uguale per GEO-005 (autorita' senza maiuscole, codice come testo);
//! 3. senza una perdita `crs_definition_not_preserved_*`, la definizione si
//!    rilegge byte per byte (una definizione che la sorgente non aveva puo'
//!    comparire: e' informazione in piu', non una perdita);
//! 4. un file che non si rilegge senza opzioni ha dichiarato la perdita di cio'
//!    che serve a rileggerlo: dell'identificatore (`crs_id_not_preserved_absent`),
//!    che `--assume-crs` riporta e la prova lo verifica, o -- se la sorgente non
//!    ne aveva uno -- della definizione (`crs_definition_not_preserved_absent`);
//! 5. una sorgente senza CRS (`missing`) si rilegge senza CRS.
//!
//! La sorgente e' cio' che `io.inspect` ne legge, non cio' che la prova crede di
//! averci scritto: cosi' la stessa regola vale al secondo passaggio, dai file
//! che il primo ha scritto (`GeoParquet`, `GeoPackage`, Shapefile, DXF, `GeoJSON`)
//! verso ogni bersaglio.
//!
//! # Perche' esiste
//!
//! La rilettura 11 ha trovato sette rilievi contro questa regola, ciascuno in una
//! conversione che la busta dichiarava `lossless` o con una perdita diversa da
//! quella vera: `EPSG:4326` che `GeoParquet` rilegge `OGC:CRS84`, un PROJJSON
//! riscritto con le chiavi riordinate e i numeri passati da `f64`, un PROJJSON
//! `EngineeringCRS` con `EPSG:4326` che il file non rilegge, un PROJJSON nella
//! colonna WKT di `gpkg_spatial_ref_sys`, un WKT anonimo che lo Shapefile
//! rilegge non risolto. Erano trovate da una matrice scritta a mano; questa e'
//! quella matrice, permanente. Su `db1307d` fallisce con trenta violazioni al
//! primo passaggio e cinquantacinque al secondo.
//!
//! `FileGDB` resta fuori: scrive solo con `gdal-backend`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const fn binario() -> &'static str {
    env!("CARGO_BIN_EXE_plenora-io")
}

// --- le definizioni ----------------------------------------------------------

const WGS84_EPSG: &str = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4326"]]"#;
const WGS84_ESRI: &str = r#"GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984",SPHEROID["WGS_1984",6378137.0,298.257223563]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]]"#;
const UTM32_EPSG: &str = r#"PROJCS["WGS 84 / UTM zone 32N",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",9],PARAMETER["scale_factor",0.9996],PARAMETER["false_easting",500000],PARAMETER["false_northing",0],UNIT["metre",1],AUTHORITY["EPSG","32632"]]"#;
const UTM32_ANONIMO: &str = r#"PROJCS["WGS 84 / UTM zone 32N",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",9],PARAMETER["scale_factor",0.9996],PARAMETER["false_easting",500000],PARAMETER["false_northing",0],UNIT["metre",1]]"#;
const WGS84_WKT2: &str = r#"GEOGCRS["WGS 84",DATUM["World Geodetic System 1984",ELLIPSOID["WGS 84",6378137,298.257223563,LENGTHUNIT["metre",1]]],PRIMEM["Greenwich",0,ANGLEUNIT["degree",0.0174532925199433]],CS[ellipsoidal,2],AXIS["geodetic latitude (Lat)",north,ORDER[1],ANGLEUNIT["degree",0.0174532925199433]],AXIS["geodetic longitude (Lon)",east,ORDER[2],ANGLEUNIT["degree",0.0174532925199433]],ID["EPSG",4326]]"#;
const CRS84_WKT2: &str = r#"GEOGCRS["WGS 84 (CRS84)",DATUM["World Geodetic System 1984",ELLIPSOID["WGS 84",6378137,298.257223563,LENGTHUNIT["metre",1]]],PRIMEM["Greenwich",0,ANGLEUNIT["degree",0.0174532925199433]],CS[ellipsoidal,2],AXIS["geodetic longitude (Lon)",east,ORDER[1],ANGLEUNIT["degree",0.0174532925199433]],AXIS["geodetic latitude (Lat)",north,ORDER[2],ANGLEUNIT["degree",0.0174532925199433]],ID["OGC","CRS84"]]"#;
const LOCALE: &str =
    r#"LOCAL_CS["x",LOCAL_DATUM["d",0],UNIT["metre",1],AXIS["X",EAST],AXIS["Y",NORTH]]"#;
const PJ_4326: &str = r#"{"$schema":"https://proj.org/schemas/v0.5/projjson.schema.json","type":"GeographicCRS","name":"WGS 84","datum":{"type":"GeodeticReferenceFrame","name":"World Geodetic System 1984","ellipsoid":{"name":"WGS 84","semi_major_axis":6378137,"inverse_flattening":298.257223563}},"coordinate_system":{"subtype":"ellipsoidal","axis":[{"name":"Geodetic latitude","abbreviation":"Lat","direction":"north","unit":"degree"},{"name":"Geodetic longitude","abbreviation":"Lon","direction":"east","unit":"degree"}]},"id":{"authority":"EPSG","code":4326}}"#;
/// Il PROJJSON di EPSG:32632, con le chiavi **fuori** dall'ordine
/// lessicografico (`type` prima di `name`): una riscrittura le riordinerebbe.
const PJ_32632: &str = r#"{"$schema":"https://proj.org/schemas/v0.5/projjson.schema.json","type":"ProjectedCRS","name":"WGS 84 / UTM zone 32N","base_crs":{"name":"WGS 84","datum":{"type":"GeodeticReferenceFrame","name":"World Geodetic System 1984","ellipsoid":{"name":"WGS 84","semi_major_axis":6378137,"inverse_flattening":298.257223563}},"coordinate_system":{"subtype":"ellipsoidal","axis":[{"name":"Geodetic latitude","abbreviation":"Lat","direction":"north","unit":"degree"},{"name":"Geodetic longitude","abbreviation":"Lon","direction":"east","unit":"degree"}]}},"conversion":{"name":"UTM zone 32N","method":{"name":"Transverse Mercator","id":{"authority":"EPSG","code":9807}},"parameters":[{"name":"Latitude of natural origin","value":0,"unit":"degree"},{"name":"Longitude of natural origin","value":9,"unit":"degree"},{"name":"Scale factor at natural origin","value":0.9996,"unit":"unity"},{"name":"False easting","value":500000,"unit":"metre"},{"name":"False northing","value":0,"unit":"metre"}]},"coordinate_system":{"subtype":"Cartesian","axis":[{"name":"Easting","abbreviation":"E","direction":"east","unit":"metre"},{"name":"Northing","abbreviation":"N","direction":"north","unit":"metre"}]},"id":{"authority":"EPSG","code":32632}}"#;

/// Lo stesso PROJJSON senza `id`.
fn pj_senza_id() -> String {
    PJ_32632.replace(r#","id":{"authority":"EPSG","code":32632}}"#, "}")
}

/// Numeri che `f64` non rappresenta, un esponente, una barra in escape: tutti
/// testi che una riscrittura cambia.
fn pj_numeri() -> String {
    let barra = char::from(0x5c_u8);
    PJ_32632
        .replace(
            r#""value":0.9996"#,
            r#""value":0.99960000000000000000000001"#,
        )
        .replace(r#""value":500000"#, r#""value":500000.00000000000000001"#)
        .replace(r#""value":9,"#, r#""value":9E0,"#)
        .replace(
            r#""name":"WGS 84 / UTM"#,
            &format!(r#""name":"WGS 84 {barra}u002F UTM"#),
        )
}

/// Un `EngineeringCRS` con `id` EPSG:4326: genere contraddetto.
const PJ_ENG_4326: &str = r#"{"$schema":"https://proj.org/schemas/v0.7/projjson.schema.json","type":"EngineeringCRS","name":"x","datum":{"name":"d"},"coordinate_system":{"subtype":"Cartesian","axis":[{"name":"X","abbreviation":"X","direction":"east","unit":"metre"},{"name":"Y","abbreviation":"Y","direction":"north","unit":"metre"}]},"id":{"authority":"EPSG","code":4326}}"#;

// --- le sorgenti -------------------------------------------------------------

const GEO: (f64, f64) = (9.0, 45.0);
const UTM: (f64, f64) = (500_000.0, 5_000_000.0);

struct Caso {
    nome: &'static str,
    risoluzione: &'static str,
    crs_id: Option<&'static str>,
    definizione: Option<(String, &'static str)>,
    assi: Option<&'static str>,
    punto: (f64, f64),
}

fn risolto(
    nome: &'static str,
    crs_id: Option<&'static str>,
    definizione: Option<(&str, &'static str)>,
    assi: &'static str,
    punto: (f64, f64),
) -> Caso {
    Caso {
        nome,
        risoluzione: "resolved",
        crs_id,
        definizione: definizione.map(|(testo, formato)| (testo.to_owned(), formato)),
        assi: Some(assi),
        punto,
    }
}

// Una riga per caso: la lunghezza e' quella della matrice, non logica.
#[allow(clippy::too_many_lines)]
fn casi() -> Vec<Caso> {
    let pj_senza_id = pj_senza_id();
    let pj_numeri = pj_numeri();
    vec![
        risolto(
            "4326_wkt",
            Some("EPSG:4326"),
            Some((WGS84_EPSG, "wkt")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "4326_esri",
            Some("EPSG:4326"),
            Some((WGS84_ESRI, "wkt")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "32632_wkt",
            Some("EPSG:32632"),
            Some((UTM32_EPSG, "wkt")),
            "easting_northing",
            UTM,
        ),
        risolto(
            "anonimo_wkt",
            None,
            Some((UTM32_ANONIMO, "wkt")),
            "easting_northing",
            UTM,
        ),
        risolto(
            "4326_pj",
            Some("EPSG:4326"),
            Some((PJ_4326, "projjson")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "32632_pj",
            Some("EPSG:32632"),
            Some((PJ_32632, "projjson")),
            "easting_northing",
            UTM,
        ),
        risolto(
            "32632_pj_numeri",
            Some("EPSG:32632"),
            Some((&pj_numeri, "projjson")),
            "easting_northing",
            UTM,
        ),
        risolto("4326_id", Some("EPSG:4326"), None, "lon_lat", GEO),
        risolto("32632_id", Some("EPSG:32632"), None, "unknown", UTM),
        risolto("crs84_id", Some("OGC:CRS84"), None, "lon_lat", GEO),
        risolto(
            "4326_wkt2",
            Some("EPSG:4326"),
            Some((WGS84_WKT2, "wkt2")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "crs84_wkt2",
            Some("OGC:CRS84"),
            Some((CRS84_WKT2, "wkt2")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "4326_locale",
            Some("EPSG:4326"),
            Some((LOCALE, "wkt")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "anonimo_pj",
            None,
            Some((&pj_senza_id, "projjson")),
            "easting_northing",
            UTM,
        ),
        risolto(
            "crs84_esri",
            Some("OGC:CRS84"),
            Some((WGS84_ESRI, "wkt")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "32632_wkt_anonimo",
            Some("EPSG:32632"),
            Some((UTM32_ANONIMO, "wkt")),
            "easting_northing",
            UTM,
        ),
        risolto(
            "crs84_locale",
            Some("OGC:CRS84"),
            Some((LOCALE, "wkt")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "4326_pj_ingegneria",
            Some("EPSG:4326"),
            Some((PJ_ENG_4326, "projjson")),
            "lon_lat",
            GEO,
        ),
        risolto(
            "3857_geografico",
            Some("EPSG:3857"),
            Some((WGS84_ESRI, "wkt")),
            "lon_lat",
            GEO,
        ),
        Caso {
            nome: "mancante",
            risoluzione: "missing",
            crs_id: None,
            definizione: None,
            assi: None,
            punto: GEO,
        },
    ]
}

fn punto_wkb((x, y): (f64, f64)) -> Vec<u8> {
    let mut byte = vec![1_u8];
    byte.extend_from_slice(&1_u32.to_le_bytes());
    byte.extend_from_slice(&x.to_le_bytes());
    byte.extend_from_slice(&y.to_le_bytes());
    byte
}

fn sorgente_ipc(dir: &Path, caso: &Caso) -> PathBuf {
    use arrow_array::{ArrayRef, BinaryArray, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};

    let mut geometria: HashMap<String, String> = [
        ("ARROW:extension:name", "geoarrow.wkb"),
        ("plenora.field_id", "1"),
        ("plenora.geometry.encoding", "wkb"),
        ("plenora.geometry.dimensions", "xy"),
        ("plenora.geometry.spatial_semantics", "geometry"),
        ("plenora.geometry.precision", "float64"),
        ("plenora.geometry.types_declaration", "exact"),
        ("plenora.geometry.types", "point"),
        ("plenora.geometry.crs_resolution", caso.risoluzione),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    if let Some(assi) = caso.assi {
        geometria.insert("plenora.geometry.axis_order".to_owned(), assi.to_owned());
    }
    if let Some(id) = caso.crs_id {
        geometria.insert("plenora.geometry.crs_id".to_owned(), id.to_owned());
        geometria.insert("crs".to_owned(), id.to_owned());
    }
    if let Some((testo, formato)) = &caso.definizione {
        geometria.insert("plenora.geometry.crs_definition".to_owned(), testo.clone());
        geometria.insert(
            "plenora.geometry.crs_definition_format".to_owned(),
            (*formato).to_owned(),
        );
    }
    let schema = Arc::new(Schema::new_with_metadata(
        vec![
            Field::new("nome", DataType::Utf8, false),
            Field::new("geometry", DataType::Binary, true).with_metadata(geometria),
        ],
        HashMap::from([("plenora.contract.version".to_owned(), "1".to_owned())]),
    ));
    let wkb = [Some(punto_wkb(caso.punto))];
    let colonne: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec!["alfa"])),
        Arc::new(wkb.iter().map(Option::as_deref).collect::<BinaryArray>()),
    ];
    let batch = RecordBatch::try_new(Arc::clone(&schema), colonne).expect("batch");
    let percorso = dir.join(format!("{}.arrow", caso.nome));
    let file = std::fs::File::create(&percorso).expect("sorgente creata");
    let mut scrittore = arrow_ipc::writer::FileWriter::try_new(file, &schema).expect("writer");
    scrittore.write(&batch).expect("batch scritto");
    scrittore.finish().expect("sorgente completata");
    percorso
}

// --- il binario --------------------------------------------------------------

/// I bersagli: il formato e il nome del file.
const BERSAGLI: [(&str, &str); 9] = [
    ("ipc", "x.arrow"),
    ("geoparquet", "x.parquet"),
    ("gpkg", "x.gpkg"),
    ("shp", "x.shp.d"),
    ("dxf", "x.dxf"),
    ("geojson", "x.geojson"),
    ("kml", "x.kml"),
    ("csv", "x.csv"),
    ("xls", "x.xlsx"),
];

/// I formati da cui parte il secondo passaggio.
const RILETTI: [&str; 5] = ["geoparquet", "gpkg", "shp", "dxf", "geojson"];

fn cli(argomenti: &[&str]) -> (bool, serde_json::Value) {
    let output = Command::new(binario())
        .args(argomenti)
        .output()
        .expect("il binario si esegue");
    let busta = serde_json::from_str(String::from_utf8_lossy(&output.stdout).trim())
        .expect("un documento JSON su stdout");
    (output.status.success(), busta)
}

/// Il CRS che `io.inspect` legge: risoluzione, identificatore, definizione.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Crs {
    stato: String,
    id: Option<String>,
    definizione: Option<String>,
}

fn crs_letto(percorso: &Path, opzioni: &[&str]) -> Result<Crs, String> {
    let mut argomenti = vec!["inspect", percorso.to_str().expect("percorso")];
    argomenti.extend_from_slice(opzioni);
    let (riuscita, busta) = cli(&argomenti);
    if !riuscita {
        return Err(format!("{}", busta["error"]["category"]));
    }
    let geometria = &busta["result"]["layers"][0]["geometry"];
    let risoluzione = &geometria["crs_resolution"];
    let testo = |valore: &serde_json::Value| valore.as_str().map(str::to_owned);
    Ok(Crs {
        stato: testo(&risoluzione["status"]).expect("la risoluzione porta lo stato"),
        id: testo(&risoluzione["id"]),
        definizione: testo(&risoluzione["definition"]),
    })
}

/// Le perdite CRS dichiarate, e la busta per la diagnostica.
fn converti(
    sorgente: &Path,
    da: &str,
    uscita: &Path,
    verso: &str,
) -> Option<(Vec<String>, String)> {
    let (riuscita, busta) = cli(&[
        "convert",
        sorgente.to_str().expect("percorso"),
        uscita.to_str().expect("percorso"),
        "--from",
        da,
        "--to",
        verso,
    ]);
    if !riuscita {
        return None;
    }
    let perdite = busta["result"]["write_loss"]["counts"]
        .as_array()
        .expect("una scrittura riuscita porta `counts`")
        .iter()
        .filter_map(|voce| voce["categoria"].as_str())
        .filter(|c| c.starts_with("crs_"))
        .map(str::to_owned)
        .collect();
    Some((
        perdite,
        busta["result"]["conversion_fidelity"]["level"].to_string(),
    ))
}

fn stesso_identificatore(a: &str, b: &str) -> bool {
    match (a.split_once(':'), b.split_once(':')) {
        (Some((aa, ca)), Some((ab, cb))) => aa.eq_ignore_ascii_case(ab) && ca == cb,
        _ => false,
    }
}

/// I bersagli che non portano un CRS: il lettore lo chiede sempre con
/// `--assume-crs` (e la geometria con `wkt_column`). Per loro la regola e'
/// che la perdita di ogni rappresentazione sia dichiarata.
const SENZA_CRS: [&str; 2] = ["csv", "xls"];

/// Le violazioni dell'invariante per una conversione riuscita.
fn violazioni(sorgente: &Crs, perdite: &[String], uscita: &Path, verso: &str) -> Vec<String> {
    let dichiara = |prefisso: &str| perdite.iter().any(|p| p.starts_with(prefisso));
    let mut errori = Vec::new();
    if SENZA_CRS.contains(&verso) {
        if sorgente.id.is_some() && !dichiara("crs_id_not_preserved_absent") {
            errori.push("identificatore non scritto e non dichiarato perduto".to_owned());
        }
        if sorgente.definizione.is_some() && !dichiara("crs_definition_not_preserved_absent") {
            errori.push("definizione non scritta e non dichiarata perduta".to_owned());
        }
        return errori;
    }
    let riletto = match crs_letto(uscita, &[]) {
        Ok(riletto) => riletto,
        Err(categoria) => {
            match &sorgente.id {
                Some(id) => {
                    if !dichiara("crs_id_not_preserved_absent") {
                        errori.push(format!(
                            "non si rilegge ({categoria}) e non dichiara crs_id_not_preserved_absent"
                        ));
                    }
                    match crs_letto(uscita, &["--assume-crs", id]) {
                        Ok(riletto)
                            if riletto
                                .id
                                .as_deref()
                                .is_some_and(|r| stesso_identificatore(r, id)) => {}
                        altro => errori.push(format!(
                            "--assume-crs non riporta l'identificatore: {altro:?}"
                        )),
                    }
                }
                None => {
                    if !dichiara("crs_definition_not_preserved_absent") {
                        errori.push(format!(
                            "non si rilegge ({categoria}), e senza identificatore non dichiara perduta la definizione"
                        ));
                    }
                }
            }
            return errori;
        }
    };
    if sorgente.stato == "missing" && riletto.stato != "missing" {
        errori.push(format!("un CRS mancante si rilegge {riletto:?}"));
    }
    if let Some(id) = &sorgente.id {
        if !dichiara("crs_id_not_preserved") && riletto.id.as_ref() != Some(id) {
            errori.push(format!(
                "identificatore conservato e riletto diverso: {:?}",
                riletto.id
            ));
        }
        if dichiara("crs_id_not_preserved_derived")
            && !riletto
                .id
                .as_deref()
                .is_some_and(|r| stesso_identificatore(r, id))
        {
            errori.push(format!(
                "identificatore derivato e riletto diverso: {:?}",
                riletto.id
            ));
        }
    }
    if let Some(definizione) = &sorgente.definizione {
        if !dichiara("crs_definition_not_preserved")
            && riletto.definizione.as_ref() != Some(definizione)
        {
            errori.push("definizione conservata e riletta diversa".to_owned());
        }
    }
    errori
}

/// Ogni bersaglio da una sorgente; restituisce i file scritti, per formato.
fn verso_ogni_bersaglio(
    dir: &Path,
    nome: &str,
    sorgente: &Path,
    da: &str,
    errori: &mut Vec<String>,
) -> Vec<(&'static str, PathBuf)> {
    let Ok(crs_sorgente) = crs_letto(sorgente, &[]) else {
        return Vec::new();
    };
    let mut scritti = Vec::new();
    for (verso, file) in BERSAGLI {
        let cartella = dir.join(format!("{nome}-{verso}"));
        std::fs::create_dir_all(&cartella).expect("cartella");
        let uscita = cartella.join(file);
        let Some((perdite, livello)) = converti(sorgente, da, &uscita, verso) else {
            continue;
        };
        for errore in violazioni(&crs_sorgente, &perdite, &uscita, verso) {
            errori.push(format!(
                "{nome} ({da}) -> {verso}: {errore}; livello {livello}, perdite {perdite:?}"
            ));
        }
        scritti.push((verso, uscita));
    }
    scritti
}

#[test]
fn ogni_scrittura_si_rilegge_come_la_dichiara() {
    let dir = tempfile::tempdir().expect("directory temporanea");
    let mut errori = Vec::new();
    for caso in casi() {
        let sorgente = sorgente_ipc(dir.path(), &caso);
        verso_ogni_bersaglio(dir.path(), caso.nome, &sorgente, "ipc", &mut errori);
    }
    assert!(errori.is_empty(), "{}", errori.join("\n"));
}

/// Il secondo passaggio: dai file che il primo ha scritto.
#[test]
fn ogni_scrittura_si_rilegge_come_la_dichiara_anche_al_secondo_passaggio() {
    let dir = tempfile::tempdir().expect("directory temporanea");
    let mut ignorati = Vec::new();
    let mut errori = Vec::new();
    for caso in casi() {
        let sorgente = sorgente_ipc(dir.path(), &caso);
        let primo = dir.path().join("primo");
        for (formato, scritto) in
            verso_ogni_bersaglio(&primo, caso.nome, &sorgente, "ipc", &mut ignorati)
        {
            if !RILETTI.contains(&formato) {
                continue;
            }
            let nome = format!("{}-{formato}", caso.nome);
            verso_ogni_bersaglio(
                &dir.path().join("secondo"),
                &nome,
                &scritto,
                formato,
                &mut errori,
            );
        }
    }
    assert!(errori.is_empty(), "{}", errori.join("\n"));
}
