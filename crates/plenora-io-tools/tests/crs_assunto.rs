//! `--assume-crs` contro il CRS che la sorgente dichiara, formato per formato.
//!
//! # Il difetto che queste prove fissano
//!
//! L'opzione la leggevano soltanto i driver che la usano (CSV, XLSX, lo
//! Shapefile senza autorita' nel `.prj`, il DXF senza `GEODATA`, il `FileGDB`
//! senza CRS). Gli altri la ignoravano: `--assume-crs EPSG:3003` su un
//! `GeoPackage` in `EPSG:4326` dava `EPSG:4326`, e la busta non diceva che
//! l'opzione era rimasta senza effetto. Il DXF con un `GEODATA` che dichiara
//! il proprio codice faceva lo stesso.
//!
//! Ora un'opzione diversa dal CRS dichiarato e' un errore `crs`, e una uguale
//! (GEO-005: autorita' senza maiuscole, codice come testo) e' ammessa.
//!
//! # Perche' passa dal binario
//!
//! Il controllo vive nell'adapter comune che ogni driver attraversa: la prova
//! sul binario mostra che nessun formato ne resta fuori, compresi quelli che
//! la CLI apre con lo stesso percorso di `read` e `convert`.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

const BINARIO: &str = env!("CARGO_BIN_EXE_plenora-io");

fn esegui(argomenti: &[&str]) -> Value {
    let uscita = Command::new(BINARIO)
        .args(argomenti)
        .args(["--format", "json"])
        .output()
        .expect("il binario parte");
    match serde_json::from_slice(&uscita.stdout) {
        Ok(documento) => documento,
        Err(errore) => panic!(
            "stdout non e' JSON ({errore}): {}",
            String::from_utf8_lossy(&uscita.stderr)
        ),
    }
}

/// I formati che dichiarano il proprio CRS, con la destinazione e il CRS che
/// ciascuno accetta in scrittura.
const DICHIARANTI: [(&str, &str, &str); 7] = [
    ("geojson", "uscita.geojson", "OGC:CRS84"),
    ("kml", "uscita.kml", "OGC:CRS84"),
    ("gpkg", "uscita.gpkg", "EPSG:4326"),
    ("geoparquet", "uscita.parquet", "EPSG:4326"),
    ("ipc", "uscita.arrow", "EPSG:4326"),
    ("dxf", "uscita.dxf", "EPSG:4326"),
    ("shp", "uscita.shp.d", "EPSG:4326"),
];

fn scrivi(dir: &Path, formato: &str, destinazione: &str, crs: &str) -> String {
    let sorgente = dir.join(format!("{formato}.csv"));
    // La nota allunga l'ingresso: il tetto dell'uscita e' proporzionale, e un
    // GeoPackage di una riga pesa piu' di mille volte un CSV di 29 byte.
    let nota = "x".repeat(200);
    std::fs::write(
        &sorgente,
        format!("nome,nota,lon,lat\nalfa,{nota},9.19,45.46\n"),
    )
    .expect("sorgente scritta");
    let uscita = dir.join(destinazione).to_str().expect("UTF-8").to_owned();
    let documento = esegui(&[
        "convert",
        sorgente.to_str().expect("UTF-8"),
        &uscita,
        "--from",
        "csv",
        "--to",
        formato,
        "--assume-crs",
        crs,
        "--in-opt",
        "x_column=lon",
        "--in-opt",
        "y_column=lat",
    ]);
    assert!(
        documento.get("result").is_some(),
        "conversione verso {formato}: {documento}"
    );
    uscita
}

#[test]
fn un_assume_crs_che_contraddice_il_crs_dichiarato_e_rifiutato() {
    let dir = tempfile::tempdir().expect("tempdir");
    for (formato, destinazione, crs) in DICHIARANTI {
        let percorso = scrivi(dir.path(), formato, destinazione, crs);

        let senza = esegui(&["inspect", &percorso]);
        let Some(dichiarato) = senza["result"]["layers"][0]["geometry"]["crs"].as_str() else {
            panic!("{formato}: nessun CRS dichiarato: {senza}");
        };
        let dichiarato = dichiarato.to_owned();

        let conflitto = esegui(&["inspect", &percorso, "--assume-crs", "EPSG:3003"]);
        assert_eq!(
            conflitto["error"]["category"], "crs",
            "{formato}: un --assume-crs diverso da {dichiarato} e' stato accettato: {conflitto}"
        );
        assert!(
            !conflitto.to_string().contains("3003"),
            "{formato}: l'errore riporta l'identificatore: {conflitto}"
        );

        // Lo stesso identificatore con l'autorita' in minuscolo e' concorde,
        // e il contratto conserva la grafia della sorgente.
        let (autorita, codice) = dichiarato.split_once(':').expect("AUTORITA:CODICE");
        let concorde = format!("{}:{codice}", autorita.to_ascii_lowercase());
        let accettato = esegui(&["inspect", &percorso, "--assume-crs", &concorde]);
        assert_eq!(
            accettato["result"]["layers"][0]["geometry"]["crs"],
            dichiarato.as_str(),
            "{formato}: {accettato}"
        );

        // Anche `read`, che apre la sorgente per consegnarla.
        let consegna = dir.path().join(format!("consegna-{formato}.arrow"));
        let letto = esegui(&[
            "read",
            &percorso,
            "--output",
            consegna.to_str().expect("UTF-8"),
            "--assume-crs",
            "EPSG:3003",
        ]);
        assert_eq!(letto["error"]["category"], "crs", "{formato}: {letto}");
    }
}
