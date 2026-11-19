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
    let status = std::process::Command::new(python)
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/filegdb_enumeration.py"
        ))
        .arg(env!("CARGO_BIN_EXE_plenora-io"))
        .status()
        .expect("launch FileGDB ownership matrix");
    assert!(status.success(), "FileGDB ownership matrix failed");
}
