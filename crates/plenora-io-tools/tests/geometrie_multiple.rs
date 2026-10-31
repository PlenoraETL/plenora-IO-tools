//! Il rifiuto dei sink monogeometria attraversa la CLI e non pubblica output.
use std::path::PathBuf;
use std::process::Command;

#[test]
fn sink_monogeometria_rifiutano_con_categoria_exit_e_nessun_output() {
    let sorgente = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../driver-geoparquet/tests/fixtures/multigeometria-pyarrow.parquet");
    let dir = tempfile::tempdir().unwrap();
    for (formato, suffisso) in [
        ("gpkg", "gpkg"),
        ("geojson", "geojson"),
        ("shp", "shp.d"),
        ("dxf", "dxf"),
        ("ipc", "arrow"),
    ] {
        let uscita = dir.path().join(format!("uscita.{suffisso}"));
        let output = Command::new(env!("CARGO_BIN_EXE_plenora-io"))
            .arg("convert")
            .arg(&sorgente)
            .arg(&uscita)
            .args(["--from", "geoparquet", "--to", formato, "--format", "json"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(3),
            "{suffisso}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(output.stderr.is_empty());
        let busta: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(busta["error"]["category"], "unsupported", "{busta}");
        assert_eq!(busta["error"]["code"], "UNSUPPORTED", "{busta}");
        assert!(!uscita.exists());
    }
}
