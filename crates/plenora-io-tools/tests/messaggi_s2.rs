use plenora_io_model::CancellationToken;
use plenora_io_tools::esegui;

fn run(args: &[&str]) -> Result<serde_json::Value, (i32, serde_json::Value)> {
    esegui(
        &args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
        &CancellationToken::new(),
    )
    .1
}

#[test]
fn structural_checks_preserve_specific_text_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    for (index, bytes, expected) in [
        (&b"name\ncaf\xe9\n"[..], "richiesto testo UTF-8"),
        (&b"\xff\xfea\0\n\0"[..], "richiesto testo UTF-8"),
        (
            &b"a,b\n1\n"[..],
            "numero di campi diverso dall'intestazione",
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (b, m))| (i, b, m))
    {
        let path = root.path().join(format!("bad-{index}.csv"));
        std::fs::write(&path, bytes).unwrap();
        let (_, doc) = run(&["read", path.to_str().unwrap()]).unwrap_err();
        assert_eq!(doc["error"]["category"], "data_mapping", "{doc}");
        assert!(
            doc["error"]["message"].as_str().unwrap().contains(expected),
            "{doc}"
        );
    }
    let path = root.path().join("delimiter.csv");
    std::fs::write(&path, "geom;id\nPOINT (1 2);1\n").unwrap();
    let (_, doc) = run(&["read", path.to_str().unwrap(), "--opt", "wkt_column=geom"]).unwrap_err();
    assert_eq!(
        doc["error"]["message"],
        "colonna WKT assente dall'intestazione"
    );
    let naked = root.path().join("naked.geojson");
    std::fs::write(&naked, r#"{"type":"Point","coordinates":[1,2]}"#).unwrap();
    let (_, doc) = run(&["read", naked.to_str().unwrap()]).unwrap_err();
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .contains("richiesta FeatureCollection"),
        "{doc}"
    );
}

#[test]
fn output_and_dxf_row_quotas_keep_phase_and_details() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.geojson");
    std::fs::write(&source, r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[1,2]}}]}"#).unwrap();
    let ipc = root.path().join("source.arrow");
    run(&[
        "read",
        source.to_str().unwrap(),
        "--output",
        ipc.to_str().unwrap(),
    ])
    .unwrap();
    let output = root.path().join("output.arrow");
    let (_, doc) = run(&[
        "write",
        ipc.to_str().unwrap(),
        output.to_str().unwrap(),
        "--to",
        "ipc",
        "--max-output-bytes",
        "1",
    ])
    .unwrap_err();
    assert_eq!(doc["error"]["category"], "resource_limit", "{doc}");
    assert_eq!(doc["error"]["phase"], "write", "{doc}");
    assert_eq!(
        doc["error"]["details"]["quota"], "max_output_bytes",
        "{doc}"
    );
    assert_eq!(doc["error"]["details"]["limit"], 1, "{doc}");
    assert!(!output.exists());

    let dxf = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/canoniche/canonico.dxf");
    let (_, doc) = run(&[
        "read",
        dxf.to_str().unwrap(),
        "--assume-crs",
        "EPSG:4326",
        "--max-rows",
        "1",
        "--layer",
        "0",
    ])
    .unwrap_err();
    assert_eq!(doc["error"]["category"], "resource_limit", "{doc}");
    assert_eq!(doc["error"]["phase"], "read", "{doc}");
    assert_eq!(doc["error"]["details"]["quota"], "max_rows", "{doc}");
    assert_eq!(doc["error"]["details"]["observed"], 2, "{doc}");
    assert_eq!(doc["error"]["details"]["limit"], 1, "{doc}");
}

#[test]
fn help_after_source_is_help() {
    assert!(run(&["read", "missing.geojson", "--help"])
        .unwrap()
        .is_null());
    assert!(plenora_io_tools::AIUTO.contains("--version, -V"));
}
