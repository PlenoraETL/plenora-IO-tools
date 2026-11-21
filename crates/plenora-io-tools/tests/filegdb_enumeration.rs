//! Ownership matrix with a rule-derived oracle and real OS access denial.
#[test]
#[cfg(feature = "gdal-backend")]
fn generated_filegdb_ownership_matrix() {
    let python = std::env::var_os("PLENORA_ENUM_PYTHON").unwrap_or_else(|| {
        if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        }
    });
    let output = std::process::Command::new(python)
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/filegdb_enumeration.py"
        ))
        .arg(env!("CARGO_BIN_EXE_plenora-io"))
        .output()
        .expect("launch FileGDB ownership matrix");
    assert!(
        output.status.success(),
        "FileGDB ownership matrix failed ({}):\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
#[cfg(feature = "gdal-backend")]
fn generated_filegdb_archive_ownership_matrix() {
    let python = std::env::var_os("PLENORA_ENUM_PYTHON").unwrap_or_else(|| "python".into());
    let output = std::process::Command::new(python)
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/filegdb_enumeration.py"))
        .arg(env!("CARGO_BIN_EXE_plenora-io"))
        .env("PLENORA_FILEGDB_ARCHIVE", "1")
        .env("PLENORA_FILEGDB_ENUM_OUTPUT", concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/filegdb-archive-enumeration"))
        .output().expect("launch archived FileGDB ownership matrix");
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}
