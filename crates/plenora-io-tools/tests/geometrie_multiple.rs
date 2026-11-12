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
        ("csv", "csv"),
        ("xls", "xlsx"),
        ("kml", "kml"),
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

#[test]
fn cardinalita_enumera_tutti_i_driver_e_il_profilo_filegdb() {
    use plenora_io_core::geometry_cardinality::{GeometryCardinality, DRIVERS};
    let catalog = plenora_io_tools::cmd_catalog();
    let drivers = catalog["drivers"].as_array().unwrap();
    let actual: std::collections::BTreeSet<_> =
        drivers.iter().map(|d| d["id"].as_str().unwrap()).collect();
    let declared: std::collections::BTreeSet<_> = DRIVERS.iter().map(|(id, _)| *id).collect();
    assert_eq!(actual, declared);
    for (id, cardinality) in DRIVERS {
        let driver = drivers.iter().find(|d| d["id"] == *id).unwrap();
        assert_eq!(
            driver["geometry_cardinality"],
            if *cardinality == GeometryCardinality::Multiple {
                "multiple"
            } else {
                "single"
            }
        );
    }
    assert!(!plenora_io_core::geometry_cardinality::supports_multiple(
        "filegdb"
    ));
}

#[test]
fn enumeration_guards_driver_and_surface_routes() {
    use plenora_io_core::geometry_cardinality::{ARROW_SURFACES, DRIVERS};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let registered: std::collections::BTreeSet<_> = DRIVERS
        .iter()
        .map(|(id, _)| format!("driver-{id}"))
        .collect();
    let on_disk: std::collections::BTreeSet<_> = std::fs::read_dir(root.join("crates"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("driver-") && n != "driver-common")
        .collect();
    assert_eq!(registered, on_disk);
    for name in &registered {
        let source =
            std::fs::read_to_string(root.join("crates").join(name).join("src/lib.rs")).unwrap();
        assert!(
            source.contains("validate_write("),
            "{name}: bordo writer assente"
        );
        assert!(
            source.contains("with_read_budget("),
            "{name}: bordo reader assente"
        );
    }
    let adoption: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("contracts/adoption-source.json")).unwrap(),
    )
    .unwrap();
    let surfaces: std::collections::BTreeSet<_> = adoption["superfici"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        surfaces,
        ARROW_SURFACES.iter().map(|(id, _, _)| *id).collect()
    );
    for (id, path, route) in ARROW_SURFACES {
        assert!(
            std::fs::read_to_string(root.join(path))
                .unwrap()
                .contains(route),
            "{id}: percorso Arrow condiviso assente"
        );
    }
    for (path, route) in [
        (
            "crates/plenora-io-core/src/capabilities.rs",
            "primary::validate_contract",
        ),
        ("crates/plenora-io-core/src/request.rs", "primary::emit"),
        (
            "crates/plenora-io-model/src/vocabolario.rs",
            "primary::PrimaryState",
        ),
        ("crates/driver-ipc/src/lib.rs", "primary::emit"),
        ("crates/driver-geoparquet/src/lib.rs", "primary::emit"),
    ] {
        assert!(
            std::fs::read_to_string(root.join(path))
                .unwrap()
                .contains(route),
            "{path}"
        );
    }
}

#[test]
fn filegdb_profile_rejects_multi_before_backend_or_publication() {
    use plenora_io_core::{
        FormatDriver, ReadOptions, Sink, Source, WriteLayer, WriteOptions, WritePlan,
    };
    use plenora_io_model::budget::PipelineBudget;
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../driver-geoparquet/tests/fixtures/multigeometria-pyarrow.parquet");
    let read =
        ReadOptions::from_read_parts(PipelineBudget::builder().build().unwrap().into_read_parts());
    let dataset = driver_geoparquet::GeoParquetDriver
        .open(Source::Path(source), read)
        .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "l".into(),
            contract: dataset.layers()[0].contract.clone(),
        }],
    };
    let opts = WriteOptions::from_write_parts(
        PipelineBudget::builder()
            .build()
            .unwrap()
            .into_write_parts(),
    );
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("rejected.gdb");
    let error = driver_filegdb::FileGdbDriver
        .create(Sink::Path(dest.clone()), &plan, &opts)
        .err()
        .unwrap();
    assert_eq!(error.category, plenora_io_model::ErrorCategory::Unsupported);
    assert_eq!(error.phase, plenora_io_model::ErrorPhase::Validate);
    assert!(error.message.contains("colonne geometriche"));
    assert!(!dest.exists());
}
