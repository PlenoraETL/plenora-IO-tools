use plenora_io_model::{
    quota::{Quota, QUOTAS},
    CancellationToken,
};
use plenora_io_tools::esegui;

fn run(args: &[&str]) -> Result<serde_json::Value, (i32, serde_json::Value)> {
    esegui(
        &args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
        &CancellationToken::new(),
    )
    .1
}

#[test]
fn every_cli_quota_rejects_zero_negative_text_and_overflow() {
    for rule in QUOTAS {
        let Some(flag) = rule.cli else { continue };
        for invalid in ["0", "-1", "abc", "18446744073709551616"] {
            let (exit, doc) = run(&["read", "missing.csv", flag, invalid]).unwrap_err();
            assert_eq!(exit, 2, "{flag} {invalid}: {doc}");
            assert_eq!(doc["error"]["category"], "invalid_configuration", "{doc}");
            assert_eq!(doc["error"]["code"], "CLI_USAGE", "{doc}");
            assert!(
                doc["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains(rule.name),
                "{doc}"
            );
        }
    }
}

#[test]
fn inconsistent_memory_explains_which_quota_to_lower() {
    for extra in [
        vec!["--memory-bytes", "33554432"],
        vec!["--max-wkb-depth", "33"],
    ] {
        let mut args = vec!["read", "missing.csv"];
        args.extend(extra);
        let (_, doc) = run(&args).unwrap_err();
        assert_eq!(doc["error"]["category"], "invalid_configuration");
        assert_eq!(doc["error"]["code"], "CLI_USAGE");
        if args.contains(&"--memory-bytes") {
            assert!(doc["error"]["message"]
                .as_str()
                .unwrap()
                .contains("abbassare --max-wkb-cell-bytes"));
        }
    }
}

#[test]
fn small_csv_to_gpkg_defaults_and_output_quota_identity() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("utm.csv");
    let output = root.path().join("out.gpkg");
    std::fs::write(&input, "id,x,y\n1,500000,5000000\n").unwrap();
    let args = [
        "convert",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
        "--from",
        "csv",
        "--to",
        "gpkg",
        "--assume-crs",
        "EPSG:32632",
        "--in-opt",
        "x_column=x",
        "--in-opt",
        "y_column=y",
    ];
    run(&args).unwrap();
    assert!(output.metadata().unwrap().len() > 26000);
    std::fs::remove_file(&output).unwrap();
    for (quota, value) in [
        (Quota::OutputExpansionRatio, "1000"),
        (Quota::OutputBytes, "26000"),
    ] {
        let mut limited = args.to_vec();
        limited.extend([quota.rule().cli.unwrap(), value]);
        let (_, doc) = run(&limited).unwrap_err();
        assert_eq!(doc["error"]["category"], "resource_limit", "{doc}");
        assert_eq!(doc["error"]["phase"], "finalize", "{doc}");
        assert!(
            doc["error"]["message"]
                .as_str()
                .unwrap()
                .contains(quota.rule().name),
            "{quota:?}: {doc}"
        );
        assert!(!output.exists());
    }
}

#[test]
fn geojson_geometry_quotas_keep_their_names_and_read_phase() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("line.geojson");
    std::fs::write(&input, r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[0,0],[1,1],[2,2]]}}]}"#).unwrap();
    let ipc = root.path().join("line.arrow");
    run(&[
        "convert",
        input.to_str().unwrap(),
        ipc.to_str().unwrap(),
        "--from",
        "geojson",
        "--to",
        "ipc",
    ])
    .unwrap();
    for source in [&input, &ipc] {
        for quota in [
            Quota::Vertices,
            Quota::WkbComponents,
            Quota::WkbCellBytes,
            Quota::GeometryComponents,
        ] {
            let (_, doc) = run(&[
                "read",
                source.to_str().unwrap(),
                quota.rule().cli.unwrap(),
                "1",
            ])
            .unwrap_err();
            assert_eq!(doc["error"]["category"], "resource_limit", "{doc}");
            assert_eq!(doc["error"]["phase"], "read", "{doc}");
            assert!(
                doc["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains(quota.rule().name),
                "{quota:?}: {doc}"
            );
        }
    }
}

#[test]
fn geometry_depth_has_its_own_quota_and_small_memory_is_usable() {
    let root = tempfile::tempdir().unwrap();
    let geojson = root.path().join("nested.geojson");
    std::fs::write(&geojson, r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":{"type":"GeometryCollection","geometries":[{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2]}]}]}}]}"#).unwrap();
    let (_, doc) = run(&["read", geojson.to_str().unwrap(), "--max-wkb-depth", "1"]).unwrap_err();
    assert_eq!(doc["error"]["category"], "resource_limit", "{doc}");
    assert_eq!(doc["error"]["phase"], "read", "{doc}");
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .contains(Quota::WkbDepth.rule().name),
        "{doc}"
    );
    run(&[
        "read",
        geojson.to_str().unwrap(),
        "--memory-bytes",
        "33554432",
        "--max-wkb-cell-bytes",
        "1048576",
    ])
    .unwrap();
}
