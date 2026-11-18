use plenora_io_model::error_table::Situation;
use plenora_io_model::CancellationToken;
use plenora_io_tools::esegui;

const DRIVERS: &[(&str, &str)] = &[
    ("csv", "csv"),
    ("geojson", "geojson"),
    ("kml", "kml"),
    ("shp", "shp"),
    ("gpkg", "gpkg"),
    ("geoparquet", "parquet"),
    ("ipc", "arrow"),
    ("xls", "xlsx"),
    ("dxf", "dxf"),
    ("filegdb", "gdb"),
];

#[test]
fn cli_rejects_ignored_repeated_and_excess_arguments() {
    for args in [
        vec!["read", "SECRET.csv", "extra"],
        vec!["read", "SECRET.csv", "--from", "csv"],
        vec!["inspect", "SECRET.csv", "--layer", "0"],
        vec!["layers", "SECRET.csv", "--limit", "1"],
        vec![
            "write",
            "SECRET.arrow",
            "out.csv",
            "--to",
            "csv",
            "--limit",
            "1",
        ],
        vec![
            "convert",
            "SECRET.csv",
            "out.arrow",
            "--from",
            "csv",
            "--to",
            "ipc",
            "--output",
            "unused",
        ],
        vec!["read", "SECRET.csv", "--limit", "1", "--limit", "2"],
        vec!["read", "SECRET.csv", "--opt", "x=1", "--opt", "x=2"],
        vec!["read", "SECRET.csv", "--limit", "0"],
        vec![
            "read",
            "SECRET.csv",
            "--deadline-ms",
            "18446744073709551615",
        ],
    ] {
        let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
        let (_, result) = esegui(&args, &CancellationToken::new());
        let (exit, doc) = result.unwrap_err();
        assert_eq!(exit, 2, "{args:?}: {doc}");
        assert_eq!(doc["error"]["category"], "invalid_configuration");
        assert_eq!(doc["error"]["code"], "CLI_USAGE");
        assert!(!doc["error"]["message"].as_str().unwrap().contains("SECRET"));
    }
}

#[test]
fn source_errors_follow_the_table_through_real_drivers() {
    let catalog = plenora_io_tools::cmd_catalog();
    let mut actual: Vec<_> = catalog["drivers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|driver| driver["id"].as_str().unwrap())
        .collect();
    let mut expected: Vec<_> = DRIVERS.iter().map(|&(id, _)| id).collect();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
    let root = tempfile::tempdir().unwrap();
    for &(driver, extension) in DRIVERS {
        for situation in [
            Situation::MissingSource,
            Situation::InvalidContent,
            Situation::DirectoryInsteadOfFile,
        ] {
            if driver == "filegdb" && situation == Situation::DirectoryInsteadOfFile {
                continue;
            }
            let path = root
                .path()
                .join(format!("SECRET-{situation:?}.{extension}"));
            if situation == Situation::InvalidContent {
                std::fs::write(&path, []).unwrap();
            } else if situation == Situation::DirectoryInsteadOfFile {
                std::fs::create_dir(&path).unwrap();
            }
            for command in ["read", "inspect", "layers", "convert"] {
                let mut args = vec![command.to_owned(), path.to_str().unwrap().to_owned()];
                if command == "convert" {
                    args.extend([
                        root.path().join("out.arrow").to_str().unwrap().to_owned(),
                        "--from".to_owned(),
                        driver.to_owned(),
                        "--to".to_owned(),
                        "ipc".to_owned(),
                    ]);
                }
                let (_, result) = esegui(&args, &CancellationToken::new());
                let (_, doc) = result.expect_err(&format!("{driver} {command} {situation:?}"));
                let rule = situation.rule();
                assert_eq!(
                    doc["error"]["category"],
                    serde_json::to_value(rule.category).unwrap(),
                    "{driver} {command}: {doc}"
                );
                assert_eq!(
                    doc["error"]["phase"],
                    serde_json::to_value(rule.phase).unwrap(),
                    "{driver} {command}: {doc}"
                );
                assert_eq!(
                    doc["error"]["code"], rule.wire_code,
                    "{driver} {command}: {doc}"
                );
                assert!(!doc["error"]["message"].as_str().unwrap().contains("SECRET"));
            }
        }
    }
}

#[test]
fn help_and_extension_errors_describe_the_available_surface() {
    for command in [
        "read",
        "write",
        "convert",
        "layers",
        "inspect",
        "catalog",
        "capabilities",
    ] {
        let args = vec![command.to_owned(), "--help".to_owned()];
        let result = esegui(&args, &CancellationToken::new());
        let (exit, text) = plenora_io_tools::uscita_del_processo(result);
        assert_eq!(exit, 0);
        assert!(text.contains("--spill-bytes"));
        assert!(text.contains("--output-expansion-ratio"));
    }
    let (_, result) = esegui(
        &["read".to_owned(), "SECRET.unknown".to_owned()],
        &CancellationToken::new(),
    );
    let (_, document) = result.unwrap_err();
    let message = document["error"]["message"].as_str().unwrap();
    for suffix in ["json", "arrows", "shp.d", "xlsx"] {
        assert!(message.contains(suffix), "{message}");
    }
    assert!(!message.contains("SECRET"));
}

#[test]
fn invalid_options_are_redacted_in_both_driver_directions() {
    use arrow_schema::{DataType, Field, Schema};
    use std::sync::Arc;
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input.arrow");
    let schema = Arc::new(Schema::new(vec![Field::new(
        "SECRET_FIELD",
        DataType::Int64,
        true,
    )]));
    let file = std::fs::File::create(&input).unwrap();
    let mut writer = arrow_ipc::writer::FileWriter::try_new(file, &schema).unwrap();
    writer.finish().unwrap();
    drop(writer);
    for &(driver, extension) in DRIVERS {
        for direction in ["read", "write"] {
            let mut args = if direction == "read" {
                vec![
                    "read".to_owned(),
                    root.path()
                        .join(format!("missing.{extension}"))
                        .to_str()
                        .unwrap()
                        .to_owned(),
                    "--opt".to_owned(),
                ]
            } else {
                vec![
                    "write".to_owned(),
                    input.to_str().unwrap().to_owned(),
                    root.path()
                        .join(format!("out.{extension}"))
                        .to_str()
                        .unwrap()
                        .to_owned(),
                    "--to".to_owned(),
                    driver.to_owned(),
                    "--out-opt".to_owned(),
                ]
            };
            args.push("SECRET_KEY=SECRET_VALUE".to_owned());
            let (_, result) = esegui(&args, &CancellationToken::new());
            let (_, doc) = result.expect_err(driver);
            let rule = Situation::InvalidConfiguration.rule();
            assert_eq!(
                doc["error"]["category"],
                serde_json::to_value(rule.category).unwrap(),
                "{driver} {direction}: {doc}"
            );
            assert_eq!(
                doc["error"]["phase"],
                serde_json::to_value(rule.phase).unwrap(),
                "{driver} {direction}: {doc}"
            );
            assert_eq!(
                doc["error"]["code"], rule.wire_code,
                "{driver} {direction}: {doc}"
            );
            assert!(!doc["error"]["message"].as_str().unwrap().contains("SECRET"));
        }
    }
}

#[test]
fn ineffective_and_malformed_crs_assumptions_are_configuration_errors() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("plain.csv");
    std::fs::write(&source, b"value\n1\n").unwrap();
    for command in ["read", "inspect", "layers"] {
        let (_, result) = esegui(
            &[
                command.to_owned(),
                source.to_str().unwrap().to_owned(),
                "--assume-crs".to_owned(),
                "EPSG:4326".to_owned(),
            ],
            &CancellationToken::new(),
        );
        let (_, doc) = result.unwrap_err();
        let rule = Situation::IneffectiveCrsAssumption.rule();
        assert_eq!(
            doc["error"]["category"],
            serde_json::to_value(rule.category).unwrap()
        );
        assert_eq!(doc["error"]["code"], rule.wire_code);
    }
    std::fs::write(&source, b"wkt\nPOINT (0 0)\n").unwrap();
    for crs in [
        "EPSG:SECRET",
        "EPSG:4326 ",
        "EPSG:0",
        "EPSG:+4326",
        "EPSG:04326",
        "EPSG:2147483648",
    ] {
        let (_, result) = esegui(
            &[
                "read".to_owned(),
                source.to_str().unwrap().to_owned(),
                "--opt".to_owned(),
                "wkt_column=wkt".to_owned(),
                "--assume-crs".to_owned(),
                crs.to_owned(),
            ],
            &CancellationToken::new(),
        );
        let (_, doc) = result.unwrap_err();
        assert_eq!(doc["error"]["category"], "invalid_configuration", "{doc}");
        assert_eq!(doc["error"]["code"], "CLI_USAGE");
        assert!(!doc["error"]["message"].as_str().unwrap().contains("SECRET"));
    }
    let (_, result) = esegui(
        &[
            "read".to_owned(),
            source.to_str().unwrap().to_owned(),
            "--opt".to_owned(),
            "wkt_column=wkt".to_owned(),
            "--assume-crs".to_owned(),
            "EPSG:999999".to_owned(),
        ],
        &CancellationToken::new(),
    );
    assert!(
        result.is_ok(),
        "a syntactically valid declaration does not resolve a registry: {result:?}"
    );
}

#[test]
fn missing_geometry_is_checked_for_every_writer() {
    use arrow_schema::{DataType, Field, Schema};
    use std::sync::Arc;
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("properties.arrow");
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        true,
    )]));
    let batch = arrow_array::RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(arrow_array::Int64Array::from(vec![1]))],
    )
    .unwrap();
    let mut writer =
        arrow_ipc::writer::FileWriter::try_new(std::fs::File::create(&input).unwrap(), &schema)
            .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    drop(writer);
    for &(driver, extension) in DRIVERS {
        let output = root.path().join(format!("out.{extension}"));
        let args = vec![
            "write".to_owned(),
            input.to_str().unwrap().to_owned(),
            output.to_str().unwrap().to_owned(),
            "--to".to_owned(),
            driver.to_owned(),
        ];
        let (_, result) = esegui(&args, &CancellationToken::new());
        if ["csv", "xls", "ipc"].contains(&driver) {
            assert!(result.is_ok(), "{driver}: {result:?}");
        } else {
            let (_, doc) = result.expect_err(driver);
            let rule = if driver == "filegdb" {
                Situation::MissingWriteBackend.rule()
            } else {
                Situation::MissingGeometry.rule()
            };
            assert_eq!(
                doc["error"]["category"],
                serde_json::to_value(rule.category).unwrap(),
                "{driver}: {doc}"
            );
            assert_eq!(
                doc["error"]["phase"],
                serde_json::to_value(rule.phase).unwrap(),
                "{driver}: {doc}"
            );
            assert_eq!(doc["error"]["code"], rule.wire_code, "{driver}: {doc}");
            assert!(!output.exists());
        }
    }
}
