//! Independent native fixture, Rust dataset API and Python SDK parity.
#[test]
#[cfg(feature = "gdal-backend")]
fn domains_subtypes_aliases_cross_public_borders() {
    use plenora_io_core::{FormatDriver, ReadOptions, Source};
    use plenora_io_model::domains::{Code, CodeType, Domain};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("metadata.gdb");
    let python = std::env::var_os("PLENORA_ENUM_PYTHON").expect("qualification Python");
    let generated = std::process::Command::new(&python)
        .arg(root.join("scripts/genera_fixture_metadati_filegdb.py"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let dataset = driver_filegdb::FileGdbDriver
        .open(
            Source::Path(path.clone()),
            ReadOptions::from_read_parts(
                plenora_io_model::budget::PipelineBudget::builder()
                    .build()
                    .unwrap()
                    .into_read_parts(),
            ),
        )
        .unwrap();
    let domains = dataset.domini().unwrap();
    assert_eq!(domains.len(), 2);
    assert!(
        matches!(&domains["stato_è"], Domain::Coded { code_type: CodeType::Int32, values } if values.len() == 2 && values[0].code == Code::Int32(1) && values[1].description.as_deref() == Some("Chiuso 水"))
    );
    assert_eq!(
        domains["intervallo_量"],
        Domain::Range {
            min: Code::Float64(-12.5),
            max: Code::Float64(105.25),
            min_inclusive: true,
            max_inclusive: true
        }
    );
    drop(dataset);
    let tested = std::process::Command::new(python)
        .arg(root.join("sdk/python/tests/filegdb_metadata_e2e.py"))
        .arg(env!("CARGO_BIN_EXE_plenora-io"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        tested.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&tested.stdout),
        String::from_utf8_lossy(&tested.stderr)
    );
}
