//! `plenora.geometry.axis_order` dichiara l'ordine delle ordinate **come
//! memorizzate** (Arrow Geometry Semantics 1.0, GEO-001..GEO-003).
//!
//! # Il difetto che queste prove fissano
//!
//! Fino alla 5.x ogni campo `EPSG:4326` usciva `lat_lon`: era l'ordine con cui
//! il registro EPSG registra il CRS, non quello dei valori. `GeoPackage`,
//! Shapefile, `GeoParquet`, DXF e le colonne di coordinate memorizzano la
//! longitudine prima, e data-tools -- che legge la chiave come il contratto
//! dice -- rifiutava ogni calcolo su quei campi. Nel verso opposto, un campo
//! che dichiarava `lat_lon` entrava in un formato che memorizza la longitudine
//! prima senza che nessuno lo dicesse: i valori uscivano invertiti.
//!
//! # Perche' coordinate che non si scambiano
//!
//! GEO-003: i byte WKB non mostrano il proprio ordine, e due ordinate
//! scambiabili renderebbero verde anche un prodotto che le inverte. Qui la
//! longitudine e' -120, fuori dal dominio di una latitudine: dichiarazione e
//! ordinate decodificate si controllano insieme, e un'inversione si vede.
//!
//! # Perche' passa dal binario
//!
//! La dichiarazione e' cio' che il prodotto consegna sul confine: si apre il
//! file Arrow consegnato con `arrow-ipc`, non il nostro driver.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use arrow_array::{Array, BinaryArray, Int64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use serde_json::Value;

const BINARIO: &str = env!("CARGO_BIN_EXE_plenora-io");

/// La longitudine: fuori da [-90, 90], non puo' essere una latitudine.
const LONGITUDINE: f64 = -120.0;
const LATITUDINE: f64 = 45.0;

fn esegui(argomenti: &[&str]) -> (bool, Value) {
    let uscita = Command::new(BINARIO)
        .args(argomenti)
        .args(["--format", "json"])
        .output()
        .expect("il binario parte");
    let documento = serde_json::from_slice(&uscita.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout non e' JSON: {}",
            String::from_utf8_lossy(&uscita.stderr)
        )
    });
    (uscita.status.success(), documento)
}

fn percorso(dir: &Path, nome: &str) -> String {
    dir.join(nome).to_str().expect("percorso UTF-8").to_owned()
}

/// Un CSV con la longitudine nella colonna `lon` e la latitudine in `lat`.
fn csv_di_coordinate(dir: &Path) -> PathBuf {
    let sorgente = dir.join("coordinate.csv");
    std::fs::write(
        &sorgente,
        format!("nome,lon,lat\nalfa,{LONGITUDINE},{LATITUDINE}\n"),
    )
    .expect("sorgente scritta");
    sorgente
}

/// Converte il CSV di coordinate in `formato`, con `EPSG:4326` dichiarato.
fn dal_csv(dir: &Path, destinazione: &str, formato: &str, uscita: &[&str]) {
    let sorgente = csv_di_coordinate(dir);
    let mut argomenti = vec![
        "convert",
        sorgente.to_str().unwrap(),
        destinazione,
        "--from",
        "csv",
        "--to",
        formato,
        "--assume-crs",
        "EPSG:4326",
        "--in-opt",
        "x_column=lon",
        "--in-opt",
        "y_column=lat",
    ];
    argomenti.extend_from_slice(uscita);
    let (riuscita, documento) = esegui(&argomenti);
    assert!(riuscita, "conversione verso {formato}: {documento}");
}

/// Legge `sorgente` e consegna Arrow; rende i metadati del campo geometria e
/// le prime due ordinate del primo punto.
fn consegna(dir: &Path, sorgente: &str, extra: &[&str]) -> (HashMap<String, String>, (f64, f64)) {
    let uscita = dir.join(format!(
        "consegnato-{}.arrow",
        Path::new(sorgente)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("x")
    ));
    let mut argomenti = vec!["read", sorgente, "--output", uscita.to_str().unwrap()];
    argomenti.extend_from_slice(extra);
    let (riuscita, documento) = esegui(&argomenti);
    assert!(riuscita, "lettura di {sorgente}: {documento}");
    let file = std::fs::File::open(&uscita).expect("consegnato");
    let lettore = arrow_ipc::reader::FileReader::try_new(file, None).expect("ipc");
    let schema = lettore.schema();
    let indice = schema
        .fields()
        .iter()
        .position(|f| f.metadata().contains_key("plenora.geometry.encoding"))
        .expect("colonna geometria");
    let metadati: HashMap<String, String> = schema
        .field(indice)
        .metadata()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for batch in lettore {
        let batch = batch.expect("batch");
        let colonna = batch
            .column(indice)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .expect("binaria");
        if let Some(valore) = (0..colonna.len()).find(|&i| colonna.is_valid(i)) {
            return (metadati, prime_ordinate(colonna.value(valore)));
        }
    }
    panic!("{sorgente}: nessun valore geometrico");
}

/// Le prime due ordinate di un punto WKB, decodificate **a mano**: un
/// decoder del prodotto che sbagliasse come l'encoder renderebbe la prova
/// verde per costruzione.
fn prime_ordinate(wkb: &[u8]) -> (f64, f64) {
    let little = match wkb[0] {
        1 => true,
        0 => false,
        altro => panic!("byte order {altro}"),
    };
    let leggi = |da: usize| {
        let byte: [u8; 8] = wkb[da..da + 8].try_into().expect("8 byte");
        if little {
            f64::from_le_bytes(byte)
        } else {
            f64::from_be_bytes(byte)
        }
    };
    let tipo: [u8; 4] = wkb[1..5].try_into().expect("4 byte");
    let tipo = if little {
        u32::from_le_bytes(tipo)
    } else {
        u32::from_be_bytes(tipo)
    };
    assert_eq!(tipo % 1000, 1, "un punto");
    (leggi(5), leggi(13))
}

fn assi(metadati: &HashMap<String, String>) -> &str {
    metadati
        .get("plenora.geometry.axis_order")
        .map_or("<assente>", String::as_str)
}

// --- produttori -----------------------------------------------------------

/// Le colonne di coordinate: la richiesta dice quale e' `x`, e `x` e' l'est
/// (decisione 0013 dei contratti).
#[test]
fn colonne_di_coordinate_in_epsg_4326_sono_lon_lat() {
    let dir = tempfile::tempdir().expect("temporanea");
    let sorgente = csv_di_coordinate(dir.path());
    let (metadati, ordinate) = consegna(
        dir.path(),
        sorgente.to_str().unwrap(),
        &[
            "--from",
            "csv",
            "--assume-crs",
            "EPSG:4326",
            "--in-opt",
            "x_column=lon",
            "--in-opt",
            "y_column=lat",
        ],
    );
    assert_eq!(assi(&metadati), "lon_lat");
    assert_eq!(ordinate, (LONGITUDINE, LATITUDINE));
}

/// Una colonna WKT non fissa l'ordine e la richiesta non lo dichiara: il
/// produttore non lo sa, e lo dice (GEO-002).
#[test]
fn una_colonna_wkt_non_stabilisce_l_ordine() {
    let dir = tempfile::tempdir().expect("temporanea");
    let sorgente = dir.path().join("wkt.csv");
    std::fs::write(
        &sorgente,
        format!("nome,geom\nalfa,POINT({LONGITUDINE} {LATITUDINE})\n"),
    )
    .expect("sorgente");
    let (metadati, ordinate) = consegna(
        dir.path(),
        sorgente.to_str().unwrap(),
        &[
            "--from",
            "csv",
            "--assume-crs",
            "EPSG:4326",
            "--in-opt",
            "wkt_column=geom",
        ],
    );
    assert_eq!(assi(&metadati), "unknown");
    assert_eq!(ordinate, (LONGITUDINE, LATITUDINE));
}

/// `GeoPackage` memorizza la longitudine prima qualunque sia il CRS
/// (GEO-002): `EPSG:4326` letto da un `GeoPackage` e' `lon_lat`.
///
/// Il `GeoPackage` nasce da un Arrow con i valori noti, non dal CSV: il tetto
/// d'uscita che la CLI ricava da un ingresso di una riga non basta alle
/// tabelle di sistema del `GeoPackage`.
#[test]
fn geopackage_in_epsg_4326_e_lon_lat() {
    let dir = tempfile::tempdir().expect("temporanea");
    let ingresso = dir.path().join("sorgente.arrow");
    arrow_con_assi(&ingresso, "EPSG:4326", "lon_lat");
    let gpkg = percorso(dir.path(), "punti.gpkg");
    let (riuscita, documento) =
        esegui(&["write", ingresso.to_str().unwrap(), &gpkg, "--to", "gpkg"]);
    assert!(riuscita, "{documento}");
    let (metadati, ordinate) = consegna(dir.path(), &gpkg, &[]);
    assert_eq!(
        metadati.get("plenora.geometry.crs_id").map(String::as_str),
        Some("EPSG:4326")
    );
    assert_eq!(assi(&metadati), "lon_lat");
    assert_eq!(ordinate, (LONGITUDINE, LATITUDINE));
}

/// Lo Shapefile memorizza x poi y; il `.prj` WGS84 non cambia i valori.
#[test]
fn shapefile_in_epsg_4326_e_lon_lat() {
    let dir = tempfile::tempdir().expect("temporanea");
    let shp = percorso(dir.path(), "punti.shp.d");
    dal_csv(dir.path(), &shp, "shp", &[]);
    let (metadati, ordinate) = consegna(dir.path(), &shp, &[]);
    assert_eq!(assi(&metadati), "lon_lat");
    assert_eq!(ordinate, (LONGITUDINE, LATITUDINE));
}

/// Il `.prj` di EPSG:3031 come lo scrive GDAL: gli assi puntano lungo i
/// meridiani, non sono una coppia est/nord (GEO-001: `other`).
#[test]
fn shapefile_polare_dichiara_other() {
    const PRJ_3031: &str = r#"PROJCS["WGS 84 / Antarctic Polar Stereographic",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4326"]],PROJECTION["Polar_Stereographic"],PARAMETER["latitude_of_origin",-71],PARAMETER["central_meridian",0],PARAMETER["false_easting",0],PARAMETER["false_northing",0],UNIT["metre",1,AUTHORITY["EPSG","9001"]],AXIS["Easting",NORTH],AXIS["Northing",NORTH],AUTHORITY["EPSG","3031"]]"#;
    let dir = tempfile::tempdir().expect("temporanea");
    let shp = percorso(dir.path(), "polare.shp.d");
    dal_csv(dir.path(), &shp, "shp", &[]);
    std::fs::write(Path::new(&shp).join("data.prj"), PRJ_3031).expect("prj");
    let (metadati, _) = consegna(dir.path(), &shp, &[]);
    assert_eq!(
        metadati.get("plenora.geometry.crs_id").map(String::as_str),
        Some("EPSG:3031")
    );
    assert_eq!(assi(&metadati), "other");
}

/// XLSX con le colonne `x` e `y`: come il CSV.
#[test]
fn xlsx_con_colonne_di_coordinate_e_lon_lat() {
    let dir = tempfile::tempdir().expect("temporanea");
    let xlsx = percorso(dir.path(), "punti.xlsx");
    dal_csv(
        dir.path(),
        &xlsx,
        "xls",
        &["--out-opt", "geometry_encoding=xy"],
    );
    let (metadati, ordinate) = consegna(
        dir.path(),
        &xlsx,
        &[
            "--from",
            "xls",
            "--assume-crs",
            "EPSG:4326",
            "--in-opt",
            "x_column=x",
            "--in-opt",
            "y_column=y",
        ],
    );
    assert_eq!(assi(&metadati), "lon_lat");
    assert_eq!(ordinate, (LONGITUDINE, LATITUDINE));
}

/// DXF: coordinate di disegno x, y.
#[test]
fn dxf_in_epsg_4326_e_lon_lat() {
    let dir = tempfile::tempdir().expect("temporanea");
    let dxf = percorso(dir.path(), "punti.dxf");
    dal_csv(dir.path(), &dxf, "dxf", &[]);
    let (metadati, ordinate) = consegna(dir.path(), &dxf, &[]);
    assert_eq!(assi(&metadati), "lon_lat");
    assert_eq!(ordinate, (LONGITUDINE, LATITUDINE));
}

/// `GeoParquet` impone longitudine prima; il writer canonicalizza `EPSG:4326`
/// in CRS84 omettendo `crs`, e la rilettura lo dichiara.
#[test]
fn geoparquet_e_lon_lat() {
    let dir = tempfile::tempdir().expect("temporanea");
    let parquet = percorso(dir.path(), "punti.parquet");
    dal_csv(dir.path(), &parquet, "geoparquet", &[]);
    let (metadati, ordinate) = consegna(dir.path(), &parquet, &[]);
    assert_eq!(assi(&metadati), "lon_lat");
    assert_eq!(ordinate, (LONGITUDINE, LATITUDINE));
}

// --- consumatori ------------------------------------------------------------

/// Un Arrow costruito a mano: un punto, con l'ordine dichiarato `assi`.
///
/// Con `lat_lon` i valori memorizzano la latitudine prima, come il contratto
/// dice: (45, -120).
fn arrow_con_assi(percorso: &Path, crs_id: &str, assi: &str) {
    let (prima, seconda) = if assi == "lat_lon" {
        (LATITUDINE, LONGITUDINE)
    } else {
        (LONGITUDINE, LATITUDINE)
    };
    let mut wkb = vec![1_u8];
    wkb.extend_from_slice(&1_u32.to_le_bytes());
    wkb.extend_from_slice(&prima.to_le_bytes());
    wkb.extend_from_slice(&seconda.to_le_bytes());
    let metadati: HashMap<String, String> = [
        ("ARROW:extension:name", "geoarrow.wkb"),
        ("plenora.field_id", "1"),
        ("plenora.geometry.encoding", "wkb"),
        ("plenora.geometry.dimensions", "xy"),
        ("plenora.geometry.spatial_semantics", "geometry"),
        ("plenora.geometry.precision", "float64"),
        ("plenora.geometry.types_declaration", "exact"),
        ("plenora.geometry.types", "point"),
        ("plenora.geometry.crs_resolution", "resolved"),
        ("plenora.geometry.crs_id", crs_id),
        ("plenora.geometry.axis_order", assi),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let schema = Arc::new(Schema::new_with_metadata(
        vec![
            Field::new("id", DataType::Int64, false).with_metadata(HashMap::from([(
                "plenora.field_id".to_owned(),
                "0".to_owned(),
            )])),
            Field::new("geom", DataType::Binary, true).with_metadata(metadati),
        ],
        HashMap::from([("plenora.contract.version".to_owned(), "1".to_owned())]),
    ));
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1_i64])),
            Arc::new(BinaryArray::from_iter([Some(wkb.as_slice())])),
        ],
    )
    .expect("batch");
    let file = std::fs::File::create(percorso).expect("file");
    let mut scrittore = arrow_ipc::writer::FileWriter::try_new(file, &schema).expect("writer");
    scrittore.write(&batch).expect("scrittura");
    scrittore.finish().expect("chiusura");
}

/// I bersagli che memorizzano per prima l'ascissa (easting o longitudine), con
/// il CRS che ciascuno accetta.
const BERSAGLI_EST_PRIMA: [(&str, &str, &str); 8] = [
    ("gpkg", "uscita.gpkg", "EPSG:4326"),
    ("shp", "uscita.shp.d", "EPSG:4326"),
    ("geoparquet", "uscita.parquet", "EPSG:4326"),
    ("geojson", "uscita.geojson", "OGC:CRS84"),
    ("kml", "uscita.kml", "OGC:CRS84"),
    ("dxf", "uscita.dxf", "EPSG:4326"),
    ("csv", "uscita.csv", "EPSG:4326"),
    ("xls", "uscita.xlsx", "EPSG:4326"),
];

/// `lat_lon` verso un formato che memorizza la longitudine prima: il bersaglio
/// non rappresenta quell'ordine, ed e' `unsupported` prima di ogni effetto
/// (GEO-000 punto 2). Prima della correzione la scrittura riusciva e i valori
/// uscivano invertiti.
#[test]
fn lat_lon_verso_un_formato_che_memorizza_la_longitudine_prima_e_rifiutato() {
    let dir = tempfile::tempdir().expect("temporanea");
    for (formato, nome, crs) in BERSAGLI_EST_PRIMA {
        let ingresso = dir.path().join(format!("lat-lon-{formato}.arrow"));
        arrow_con_assi(&ingresso, crs, "lat_lon");
        let uscita = dir.path().join(nome);
        let (riuscita, documento) = esegui(&[
            "write",
            ingresso.to_str().unwrap(),
            uscita.to_str().unwrap(),
            "--to",
            formato,
        ]);
        assert!(!riuscita, "{formato}: lat_lon accettato: {documento}");
        assert_eq!(
            documento["error"]["category"], "unsupported",
            "{formato}: {documento}"
        );
        assert!(
            !uscita.exists(),
            "{formato}: nessuna destinazione su un rifiuto"
        );
    }
}

/// Lo stesso per `northing_easting`, l'altro ordine che questi formati non
/// rappresentano.
#[test]
fn northing_easting_verso_gpkg_e_rifiutato() {
    let dir = tempfile::tempdir().expect("temporanea");
    let ingresso = dir.path().join("ne.arrow");
    arrow_con_assi(&ingresso, "EPSG:3003", "northing_easting");
    let uscita = dir.path().join("ne.gpkg");
    let (riuscita, documento) = esegui(&[
        "write",
        ingresso.to_str().unwrap(),
        uscita.to_str().unwrap(),
        "--to",
        "gpkg",
    ]);
    assert!(!riuscita, "{documento}");
    assert_eq!(documento["error"]["category"], "unsupported", "{documento}");
    assert!(!uscita.exists());
}

/// Il controllo: `lon_lat` verso gli stessi bersagli passa, e il `GeoPackage`
/// riletto conserva le ordinate.
#[test]
fn lon_lat_verso_un_formato_che_memorizza_la_longitudine_prima_passa() {
    let dir = tempfile::tempdir().expect("temporanea");
    for (formato, nome, crs) in BERSAGLI_EST_PRIMA {
        let ingresso = dir.path().join(format!("lon-lat-{formato}.arrow"));
        arrow_con_assi(&ingresso, crs, "lon_lat");
        let uscita = dir.path().join(nome);
        let (riuscita, documento) = esegui(&[
            "write",
            ingresso.to_str().unwrap(),
            uscita.to_str().unwrap(),
            "--to",
            formato,
        ]);
        assert!(riuscita, "{formato}: {documento}");
    }
    let (metadati, ordinate) = consegna(dir.path(), &percorso(dir.path(), "uscita.gpkg"), &[]);
    assert_eq!(assi(&metadati), "lon_lat");
    assert_eq!(ordinate, (LONGITUDINE, LATITUDINE));
}

/// Arrow IPC porta la dichiarazione con i valori: `lat_lon` passa e resta
/// `lat_lon` (ARROW-008), con i valori nello stesso ordine.
#[test]
fn arrow_ipc_conserva_lat_lon() {
    let dir = tempfile::tempdir().expect("temporanea");
    let ingresso = dir.path().join("lat-lon.arrow");
    arrow_con_assi(&ingresso, "EPSG:4326", "lat_lon");
    let uscita = dir.path().join("copia.arrow");
    let (riuscita, documento) = esegui(&[
        "write",
        ingresso.to_str().unwrap(),
        uscita.to_str().unwrap(),
        "--to",
        "ipc",
    ]);
    assert!(riuscita, "{documento}");
    let (metadati, ordinate) = consegna(dir.path(), uscita.to_str().unwrap(), &[]);
    assert_eq!(assi(&metadati), "lat_lon");
    assert_eq!(ordinate, (LATITUDINE, LONGITUDINE));
}
