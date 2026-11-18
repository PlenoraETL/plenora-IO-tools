//! Owner decisions: selection never silently omits readable objects.
use serde_json::Value;
use std::{path::Path, process::Command};

fn run(args: &[&str]) -> (bool, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_plenora-io"))
        .args(args)
        .output()
        .unwrap();
    assert!(output.stderr.is_empty());
    (
        output.status.success(),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}
fn count(doc: &Value, section: &str, category: &str) -> u64 {
    doc["result"][section]["counts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["categoria"] == category)
        .map(|row| row["conteggio"].as_u64().unwrap())
        .sum()
}

#[test]
fn selection_counts_readable_objects_and_reports_every_omission() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.gpkg");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/canoniche/canonico.gpkg"),
        &source,
    )
    .unwrap();
    let conn = rusqlite::Connection::open(&source).unwrap();
    conn.execute_batch("ALTER TABLE secondario RENAME TO aaa; UPDATE gpkg_contents SET table_name='aaa',identifier='aaa' WHERE table_name='secondario';").unwrap();
    drop(conn);
    let src = source.to_str().unwrap();
    let (ok, doc) = run(&["read", src]);
    assert!(ok, "{doc}");
    assert_eq!(doc["result"]["rows_read"], 5);
    let out = dir.path().join("one.gpkg");
    let (ok, doc) = run(&[
        "convert",
        src,
        out.to_str().unwrap(),
        "--from",
        "gpkg",
        "--to",
        "gpkg",
    ]);
    assert!(ok, "{doc}");
    assert_eq!(count(&doc, "read_loss", "source_object_not_readable"), 1);
    assert_eq!(doc["result"]["layers"].as_array().unwrap().len(), 1);
    let conn = rusqlite::Connection::open(&source).unwrap();
    conn.execute_batch("CREATE TABLE zsecondo AS SELECT * FROM principale;
        INSERT INTO gpkg_contents(table_name,data_type,identifier,srs_id) SELECT 'zsecondo',data_type,'zsecondo',srs_id FROM gpkg_contents WHERE table_name='principale';
        INSERT INTO gpkg_geometry_columns SELECT 'zsecondo',column_name,geometry_type_name,srs_id,z,m FROM gpkg_geometry_columns WHERE table_name='principale';").unwrap();
    drop(conn);
    let (ok, doc) = run(&["read", src]);
    assert!(!ok);
    assert_eq!(doc["error"]["code"], "LAYER_SELECTION_REQUIRED");
    assert_eq!(
        doc["error"]["details"]["objects"].as_array().unwrap().len(),
        3
    );
    let out = dir.path().join("ambiguous.arrow");
    let (ok, doc) = run(&[
        "convert",
        src,
        out.to_str().unwrap(),
        "--from",
        "gpkg",
        "--to",
        "ipc",
    ]);
    assert!(!ok);
    assert_eq!(doc["error"]["code"], "LAYER_SELECTION_REQUIRED");
    assert!(!out.exists());
    let out = dir.path().join("both.gpkg");
    let (ok, doc) = run(&[
        "convert",
        src,
        out.to_str().unwrap(),
        "--from",
        "gpkg",
        "--to",
        "gpkg",
    ]);
    assert!(ok, "{doc}");
    assert_eq!(doc["result"]["layers"].as_array().unwrap().len(), 2);
    assert_eq!(doc["result"]["total_rows"], 10);
    assert_eq!(count(&doc, "read_loss", "source_object_not_readable"), 1);
}

#[test]
fn numeric_declarations_match_observed_reconstructed_values_in_both_directions() {
    let dir = tempfile::tempdir().unwrap();
    for (index, (text, changed)) in [
        ("1.5", false),
        ("1.00000000000000001", true),
        ("2.000e0", false),
    ]
    .into_iter()
    .enumerate()
    {
        for format in ["geojson", "kml", "csv"] {
            let path = dir.path().join(format!("{index}.{format}"));
            let content = match format {
                "geojson" => format!(r#"{{"type":"FeatureCollection","features":[{{"type":"Feature","geometry":{{"type":"Point","coordinates":[{text},2]}},"properties":{{}}}}]}}"#),
                "kml" => format!("<kml xmlns=\"http://www.opengis.net/kml/2.2\"><Placemark><Point><extrude>0</extrude><altitudeMode>clampToGround</altitudeMode><coordinates>{text},2</coordinates></Point></Placemark></kml>"),
                _ => format!("x,y\n{text},2\n"),
            };
            std::fs::write(&path, content).unwrap();
            let output = dir.path().join(format!("{index}-{format}.arrow"));
            let mut args = vec![
                "read",
                path.to_str().unwrap(),
                "--layer",
                "0",
                "--output",
                output.to_str().unwrap(),
            ];
            if format == "csv" {
                args.extend([
                    "--assume-crs",
                    "EPSG:4326",
                    "--in-opt",
                    "x_column=x",
                    "--in-opt",
                    "y_column=y",
                ]);
            }
            let (ok, doc) = run(&args);
            assert!(ok, "{doc}");
            assert_eq!(
                count(&doc, "loss", "native_number_precision"),
                u64::from(changed),
                "{format}: {text}: {doc}"
            );
            if format == "kml" {
                assert_eq!(
                    count(&doc, "loss", "native_foreign_members_not_preserved"),
                    0
                );
            }
            let batch =
                arrow_ipc::reader::FileReader::try_new(std::fs::File::open(output).unwrap(), None)
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap();
            let geometry = batch
                .column(0)
                .as_any()
                .downcast_ref::<arrow_array::BinaryArray>()
                .unwrap()
                .value(0);
            // Point WKB: endian byte + type, followed by two doubles.
            let x = f64::from_le_bytes(geometry[5..13].try_into().unwrap());
            assert_eq!(x.to_bits(), text.parse::<f64>().unwrap().to_bits());
            assert_eq!(x.to_bits() == 1.0_f64.to_bits(), changed);
        }
    }
}

#[cfg(feature = "gdal-backend")]
#[test]
fn filegdb_cli_multilayer_selection_is_explicit() {
    let dir = tempfile::tempdir().unwrap();
    use plenora_io_core::driver::{FormatDriver, ReadOptions, Sink, Source, WriteOptions};
    use plenora_io_core::{WriteLayer, WritePlan};
    use plenora_io_model::{budget::PipelineBudget, contract::LayerId};
    let driver = driver_filegdb::FileGdbDriver;
    let dataset = driver
        .open(
            Source::Path(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/canoniche/canonico.gdb"),
            ),
            ReadOptions::from_read_parts(
                PipelineBudget::builder().build().unwrap().into_read_parts(),
            ),
        )
        .unwrap();
    let layer = &dataset.layers()[0];
    let mut contract = layer.contract.clone();
    contract.schema = std::sync::Arc::new(arrow_schema::Schema::new_with_metadata(
        vec![contract.schema.field(0).clone()],
        contract.schema.metadata().clone(),
    ));
    let plan = WritePlan {
        layers: ["first", "second"]
            .into_iter()
            .map(|name| WriteLayer {
                name: name.to_owned(),
                contract: contract.clone(),
            })
            .collect(),
    };
    let native = dir.path().join("multi.gdb");
    let mut writer = driver
        .create(
            Sink::Path(native.clone()),
            &plan,
            &WriteOptions::from_write_parts(
                PipelineBudget::builder()
                    .build()
                    .unwrap()
                    .into_write_parts(),
            ),
        )
        .unwrap();
    let request = plenora_io_core::ReadRequest {
        layer: layer.id,
        projected_fields: None,
        projection_mode: plenora_io_core::ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: plenora_io_core::ReadScope::default(),
        batch_target: plenora_io_core::BatchTarget::default(),
        cancellation: Default::default(),
    };
    let mut reader = dataset.open_layer_reader(&request).unwrap();
    let mut rows = 0;
    while let Some(batch) = reader.next_batch().unwrap() {
        rows += batch.num_rows();
        let batch = arrow_array::RecordBatch::try_new(
            contract.schema.clone(),
            vec![batch.column(0).clone()],
        )
        .unwrap();
        writer.write_to_layer(LayerId(0), &batch).unwrap();
        writer.write_to_layer(LayerId(1), &batch).unwrap();
    }
    writer.finish().unwrap();
    let (ok, doc) = run(&["read", native.to_str().unwrap()]);
    assert!(!ok, "{doc}");
    assert_eq!(doc["error"]["code"], "LAYER_SELECTION_REQUIRED");
    assert_eq!(
        doc["error"]["details"]["objects"].as_array().unwrap().len(),
        2
    );
    let single = dir.path().join("single.arrow");
    let (ok, doc) = run(&[
        "convert",
        native.to_str().unwrap(),
        single.to_str().unwrap(),
        "--from",
        "filegdb",
        "--to",
        "ipc",
    ]);
    assert!(!ok, "{doc}");
    assert_eq!(doc["error"]["code"], "LAYER_SELECTION_REQUIRED");
    assert!(!single.exists());
    for index in ["0", "1"] {
        let (ok, doc) = run(&["read", native.to_str().unwrap(), "--layer", index]);
        assert!(ok, "{doc}");
        assert_eq!(doc["result"]["rows_read"], rows);
    }
    let both = dir.path().join("both.gpkg");
    let (ok, doc) = run(&[
        "convert",
        native.to_str().unwrap(),
        both.to_str().unwrap(),
        "--from",
        "filegdb",
        "--to",
        "gpkg",
    ]);
    assert!(ok, "{doc}");
    assert_eq!(doc["result"]["layers"].as_array().unwrap().len(), 2);
    assert_eq!(doc["result"]["total_rows"], rows * 2);
    assert_eq!(count(&doc, "read_loss", "source_object_not_readable"), 0);
}
