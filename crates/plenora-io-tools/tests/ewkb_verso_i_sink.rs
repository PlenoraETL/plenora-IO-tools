//! `io.write` di un dataset EWKB verso un sink che scrive WKB ISO.
//!
//! EWKB e' l'unica forma che plenora-database-tools emette leggendo `PostGIS`, e
//! `ARROW-VOCABULARY-1.0` la ammette. Fino alla 4.1.1 ogni sink tranne Arrow
//! IPC la rifiutava con `encoding: ewkb` (la catena database -> IO della suite
//! di interoperabilita'). Ora si converte in WKB ISO, esattamente, se il SRID e'
//! quello del CRS; altrimenti e' un errore esplicito.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use arrow_array::{ArrayRef, BinaryArray, Int64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use serde_json::Value;

const BINARIO: &str = env!("CARGO_BIN_EXE_plenora-io");

fn esegui(argomenti: &[&str]) -> (bool, Value) {
    let uscita = Command::new(BINARIO)
        .args(argomenti)
        .args(["--format", "json"])
        .output()
        .expect("il binario parte");
    let documento = serde_json::from_slice(&uscita.stdout).expect("stdout e' JSON");
    (uscita.status.success(), documento)
}

/// Un poligono con buco in EWKB little endian, con il SRID nella parola di tipo.
fn poligono(srid: Option<u32>, x0: f64) -> Vec<u8> {
    let anelli: [&[(f64, f64)]; 2] = [
        &[(x0, 0.1), (x0 + 1.0, 0.1), (x0 + 1.0, 1.1), (x0, 0.1)],
        &[
            (x0 + 0.2, 0.3),
            (x0 + 0.4, 0.3),
            (x0 + 0.4, 0.5),
            (x0 + 0.2, 0.3),
        ],
    ];
    let mut byte = vec![1_u8];
    let tipo: u32 = 3 | if srid.is_some() { 0x2000_0000 } else { 0 };
    byte.extend_from_slice(&tipo.to_le_bytes());
    if let Some(srid) = srid {
        byte.extend_from_slice(&srid.to_le_bytes());
    }
    byte.extend_from_slice(&2_u32.to_le_bytes());
    for anello in anelli {
        byte.extend_from_slice(&u32::try_from(anello.len()).unwrap().to_le_bytes());
        for (x, y) in anello {
            byte.extend_from_slice(&x.to_le_bytes());
            byte.extend_from_slice(&y.to_le_bytes());
        }
    }
    byte
}

/// Il dataset come lo consegna database: `encoding=ewkb`, SRID e CRS.
fn dataset(percorso: &Path, srid_metadati: &str, crs_id: &str, srid_payload: u32) {
    let metadati: HashMap<String, String> = [
        ("ARROW:extension:name", "geoarrow.wkb"),
        ("plenora.field_id", "2"),
        ("plenora.geometry.encoding", "ewkb"),
        ("plenora.geometry.dimensions", "xy"),
        ("plenora.geometry.spatial_semantics", "geometry"),
        ("plenora.geometry.precision", "float64"),
        ("plenora.geometry.types_declaration", "exact"),
        ("plenora.geometry.types", "polygon"),
        ("plenora.geometry.crs_resolution", "resolved"),
        ("plenora.geometry.crs_id", crs_id),
        ("plenora.geometry.axis_order", "lon_lat"),
        ("plenora.geometry.srid", srid_metadati),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let id_md: HashMap<String, String> =
        HashMap::from([("plenora.field_id".to_owned(), "1".to_owned())]);
    let schema = Arc::new(Schema::new_with_metadata(
        vec![
            Field::new("id", DataType::Int64, false).with_metadata(id_md),
            Field::new("geom", DataType::Binary, true).with_metadata(metadati),
        ],
        HashMap::from([("plenora.contract.version".to_owned(), "1".to_owned())]),
    ));
    let geometrie = [
        Some(poligono(Some(srid_payload), 12.0)),
        None,
        Some(poligono(Some(srid_payload), 13.0)),
    ];
    let colonne: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from(vec![1, 2, 3])),
        Arc::new(BinaryArray::from_iter(
            geometrie.iter().map(Option::as_deref),
        )),
    ];
    let batch = RecordBatch::try_new(Arc::clone(&schema), colonne).expect("batch");
    let file = std::fs::File::create(percorso).expect("file");
    let mut scrittore = arrow_ipc::writer::FileWriter::try_new(file, &schema).expect("writer");
    scrittore.write(&batch).expect("scrittura");
    scrittore.finish().expect("chiusura");
}

#[test]
fn ewkb_verso_geopackage_si_scrive_in_wkb_identico() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let ingresso = temporanea.path().join("ewkb.arrow");
    dataset(&ingresso, "4326", "EPSG:4326", 4326);
    let gpkg = temporanea.path().join("uscita.gpkg");
    let (riuscita, documento) = esegui(&[
        "write",
        ingresso.to_str().unwrap(),
        gpkg.to_str().unwrap(),
        "--to",
        "gpkg",
    ]);
    assert!(riuscita, "{documento}");
    assert_eq!(documento["result"]["rows_written"], 3);

    // Il giro si chiude rileggendo: le geometrie sono WKB ISO delle stesse
    // coordinate, cioe' l'EWKB d'ingresso senza i quattro byte del SRID.
    let riletto = temporanea.path().join("riletto.arrow");
    let (riuscita, documento) = esegui(&[
        "read",
        gpkg.to_str().unwrap(),
        "--output",
        riletto.to_str().unwrap(),
    ]);
    assert!(riuscita, "{documento}");
    let file = std::fs::File::open(&riletto).expect("riletto");
    let lettore = arrow_ipc::reader::FileReader::try_new(file, None).expect("ipc");
    let schema = lettore.schema();
    let indice = schema
        .fields()
        .iter()
        .position(|f| f.metadata().get("plenora.geometry.encoding").is_some())
        .expect("colonna geometria");
    let mut valori = Vec::new();
    for batch in lettore {
        let batch = batch.expect("batch");
        let colonna = batch
            .column(indice)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .expect("binaria")
            .clone();
        valori.extend(colonna.iter().map(|v| v.map(<[u8]>::to_vec)));
    }
    assert_eq!(
        valori,
        vec![Some(poligono(None, 12.0)), None, Some(poligono(None, 13.0))],
        "WKB ISO con le stesse coordinate, bit per bit"
    );
    let crs = schema
        .field(indice)
        .metadata()
        .get("plenora.geometry.crs_id")
        .cloned();
    assert_eq!(
        crs.as_deref(),
        Some("EPSG:4326"),
        "il CRS resta quello dello SRID"
    );
}

#[test]
fn ewkb_con_srid_diverso_dal_crs_e_un_errore_esplicito() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let ingresso = temporanea.path().join("discorde.arrow");
    dataset(&ingresso, "3003", "EPSG:4326", 3003);
    let gpkg = temporanea.path().join("uscita.gpkg");
    let (riuscita, documento) = esegui(&[
        "write",
        ingresso.to_str().unwrap(),
        gpkg.to_str().unwrap(),
        "--to",
        "gpkg",
    ]);
    assert!(!riuscita, "{documento}");
    assert!(!gpkg.exists(), "nessuna destinazione su un rifiuto");
    let messaggio = documento["error"]["message"].as_str().unwrap_or_default();
    assert!(
        messaggio.contains("SRID") || messaggio.contains("CRS"),
        "il rifiuto nomina la discordanza: {documento}"
    );
}

#[test]
fn un_payload_con_un_altro_srid_e_un_errore_esplicito() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let ingresso = temporanea.path().join("payload.arrow");
    dataset(&ingresso, "4326", "EPSG:4326", 3003);
    let gpkg = temporanea.path().join("uscita.gpkg");
    let (riuscita, documento) = esegui(&[
        "write",
        ingresso.to_str().unwrap(),
        gpkg.to_str().unwrap(),
        "--to",
        "gpkg",
    ]);
    assert!(!riuscita, "{documento}");
    assert!(!gpkg.exists(), "nessuna destinazione su un rifiuto");
}

#[test]
fn convert_da_ipc_ewkb_verso_geopackage() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let ingresso = temporanea.path().join("ewkb.arrow");
    dataset(&ingresso, "4326", "EPSG:4326", 4326);
    let gpkg = temporanea.path().join("convertito.gpkg");
    let (riuscita, documento) = esegui(&[
        "convert",
        ingresso.to_str().unwrap(),
        gpkg.to_str().unwrap(),
        "--from",
        "ipc",
        "--to",
        "gpkg",
    ]);
    assert!(riuscita, "{documento}");
}
