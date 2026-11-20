//! Generated loss qualification and retained regressions from S1d.
#[path = "loss_enumeration/generated.rs"]
mod generated;
use arrow_array::{ArrayRef, BinaryArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use plenora_io_model::{
    contract::{CoordinateDimensions, FieldId, GeometryColumnContract, GeometryType},
    crs::ResolvedCrs,
    wkb::{encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue},
};
use serde_json::Value;
use std::{fs::File, path::Path, process::Command, sync::Arc};

const DRIVERS: &[(&str, &str)] = &[
    ("geojson", "geojson"),
    ("kml", "kml"),
    ("csv", "csv"),
    ("xls", "xlsx"),
    ("ipc", "arrow"),
];

fn save(path: &Path, batch: &RecordBatch) {
    let mut writer =
        arrow_ipc::writer::FileWriter::try_new(File::create(path).unwrap(), &batch.schema())
            .unwrap();
    writer.write(batch).unwrap();
    writer.finish().unwrap();
}

fn load(path: &Path) -> RecordBatch {
    let mut reader =
        arrow_ipc::reader::FileReader::try_new(File::open(path).unwrap(), None).unwrap();
    let batch = reader
        .next()
        .map_or_else(|| RecordBatch::new_empty(reader.schema()), Result::unwrap);
    assert!(
        reader.next().is_none(),
        "fixture unexpectedly spans multiple batches"
    );
    batch
}

fn convert(source: &Path, target: &Path, from: &str, to: &str) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_plenora-io"));
    command
        .arg("convert")
        .arg(source)
        .arg(target)
        .args(["--from", from, "--to", to, "--layer", "0"]);
    if matches!(from, "csv" | "xls") {
        command.args([
            "--assume-crs",
            "OGC:CRS84",
            "--in-opt",
            "wkt_column=geometry",
        ]);
    }
    let output = command.output().unwrap();
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{from} -> {to}: {doc}");
    doc
}

fn empty_report(doc: &Value, section: &str, context: &str) {
    assert_eq!(
        doc["result"][section]["counts"],
        serde_json::json!([]),
        "{context}: {doc}"
    );
}

// Read the native contract: the IPC writer adds field identities, so comparing
// only IPC files would conceal a real change at the native reader boundary.
fn native_batch(path: &Path, ipc: &Path, driver: &str) -> RecordBatch {
    let reader: Box<dyn plenora_io_core::FormatDriver> = match driver {
        "geojson" => Box::new(driver_geojson::GeoJsonDriver),
        "kml" => Box::new(driver_kml::KmlDriver),
        "csv" => Box::new(driver_csv::CsvDriver),
        "xls" => Box::new(driver_xls::XlsDriver),
        "ipc" => Box::new(driver_ipc::IpcDriver),
        "shp" => Box::new(driver_shp::ShpDriver),
        "dxf" => Box::new(driver_dxf::DxfDriver),
        "gpkg" => Box::new(driver_gpkg::GpkgDriver),
        "geoparquet" => Box::new(driver_geoparquet::GeoParquetDriver),
        "filegdb" => Box::new(driver_filegdb::FileGdbDriver),
        _ => unreachable!(),
    };
    let mut options = plenora_io_core::ReadOptions::from_read_parts(
        plenora_io_model::budget::PipelineBudget::builder()
            .build()
            .unwrap()
            .into_read_parts(),
    );
    if matches!(driver, "csv" | "xls") {
        options = options.with_assume_crs("OGC:CRS84").with_format_options(
            std::iter::once(("wkt_column".to_owned(), "geometry".to_owned())).collect(),
        );
    }
    let dataset = reader
        .open(plenora_io_core::Source::Path(path.to_owned()), options)
        .unwrap();
    RecordBatch::try_new(
        dataset.layers()[0].contract.schema.clone(),
        load(ipc).columns().to_vec(),
    )
    .unwrap()
}

const fn coordinate(x: f64, y: f64) -> WkbCoordinate {
    WkbCoordinate {
        x,
        y,
        z: None,
        m: None,
    }
}

fn geometry_batch(value: WkbValue, kind: GeometryType) -> RecordBatch {
    let mut contract =
        GeometryColumnContract::wkb_xy(FieldId(0), "geometry", ResolvedCrs::wgs84(), true);
    contract.set_exact_geometry_types(vec![kind]);
    let field = plenora_io_model::geometry::with_geometry_contract_metadata(
        &Field::new("geometry", DataType::Binary, true)
            .with_metadata([("ARROW:extension:name".to_owned(), "geoarrow.wkb".to_owned())]),
        &contract,
    );
    let schema =
        plenora_io_model::geometry::with_contract_version(Arc::new(Schema::new(vec![field])));
    let bytes = encode_wkb(
        &WkbGeometry {
            value,
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        },
        WkbFlavor::Iso,
    )
    .unwrap();
    RecordBatch::try_new(
        schema,
        vec![Arc::new(BinaryArray::from(vec![bytes.as_slice()]))],
    )
    .unwrap()
}

const fn child(value: WkbValue) -> WkbGeometry {
    WkbGeometry {
        value,
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    }
}

#[test]
fn geometry_roundtrips_compare_values_and_both_reports() {
    exercise_geometry();
}

fn exercise_geometry() {
    let point = WkbValue::Point(coordinate(1.0, 2.0));
    let line = WkbValue::LineString(vec![coordinate(-0.0, 2.0), coordinate(3.0, 4.0)]);
    let polygon = WkbValue::Polygon(vec![vec![
        coordinate(0.0, 0.0),
        coordinate(1.0, 0.0),
        coordinate(1.0, 1.0),
        coordinate(0.0, 0.0),
    ]]);
    let cases = [
        (GeometryType::Point, point.clone()),
        (GeometryType::LineString, line.clone()),
        (GeometryType::Polygon, polygon.clone()),
        (
            GeometryType::MultiPoint,
            WkbValue::MultiPoint(vec![child(point.clone())]),
        ),
        (
            GeometryType::MultiLineString,
            WkbValue::MultiLineString(vec![child(line.clone())]),
        ),
        (
            GeometryType::MultiPolygon,
            WkbValue::MultiPolygon(vec![child(polygon)]),
        ),
        (
            GeometryType::GeometryCollection,
            WkbValue::GeometryCollection(vec![child(point), child(line)]),
        ),
    ];
    for (driver, extension) in DRIVERS {
        for (kind, value) in &cases {
            let dir = tempfile::tempdir().unwrap();
            let seed = dir.path().join("seed.arrow");
            let native = dir.path().join(format!("seed.{extension}"));
            // IPC needs a distinct path from seed.arrow.
            let native = native.with_file_name(format!("native.{extension}"));
            let normalized = dir.path().join("normalized.arrow");
            let batch = geometry_batch(value.clone(), *kind);
            save(&seed, &batch);
            convert(&seed, &native, "ipc", driver);
            let read = convert(&native, &normalized, driver, "ipc");
            let before = native_batch(&native, &normalized, driver);
            save(&normalized, &before);
            assert_eq!(batch.column(0), before.column(0), "{driver} {kind:?}");
            empty_report(&read, "read_loss", &format!("{driver} {kind:?}"));
            let native2 = dir.path().join(format!("second.{extension}"));
            let output = dir.path().join("output.arrow");
            let written = convert(&normalized, &native2, "ipc", driver);
            let read = convert(&native2, &output, driver, "ipc");
            let after = native_batch(&native2, &output, driver);
            assert_eq!(before, after, "{driver} {kind:?}");
            if matches!(*driver, "csv" | "xls") {
                // WKT carries coordinates but no CRS; the read assumption above
                // is external evidence, not a representation written to the file.
                assert_eq!(
                    written["result"]["write_loss"]["counts"],
                    serde_json::json!([
                        {"categoria":"arrow_metadata_not_preserved", "conteggio":4},
                        {"categoria":"crs_id_not_preserved_absent", "conteggio":1},
                    ]),
                    "{driver} {kind:?}: {written}"
                );
            } else {
                empty_report(&written, "write_loss", &format!("{driver} {kind:?}"));
            }
            empty_report(&read, "read_loss", &format!("{driver} {kind:?}"));
        }
    }
}

#[test]
fn attribute_roundtrips_compare_values_and_both_reports() {
    exercise_attributes();
}

fn exercise_attributes() {
    use arrow_array::{BooleanArray, Float64Array, Int64Array, StringArray};
    let cases: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![Some("text"), Some(""), None])),
        Arc::new(BooleanArray::from(vec![Some(true), Some(false), None])),
        Arc::new(Int64Array::from(vec![Some(1), Some(-2), None])),
        Arc::new(Float64Array::from(vec![Some(1.5), Some(-2.5), None])),
    ];
    for (driver, extension) in DRIVERS.iter().filter(|(d, _)| *d != "kml") {
        for values in &cases {
            let dir = tempfile::tempdir().unwrap();
            let base = geometry_batch(WkbValue::Point(coordinate(1.0, 2.0)), GeometryType::Point);
            let geometry = base
                .column(0)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap()
                .value(0);
            let schema =
                plenora_io_model::geometry::with_contract_version(Arc::new(Schema::new(vec![
                    base.schema().field(0).clone(),
                    Field::new("value", values.data_type().clone(), true),
                ])));
            let batch = RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(BinaryArray::from(vec![geometry; 3])),
                    values.clone(),
                ],
            )
            .unwrap();
            let seed = dir.path().join("seed.arrow");
            let native = dir.path().join(format!("native.{extension}"));
            let normalized = dir.path().join("normalized.arrow");
            save(&seed, &batch);
            convert(&seed, &native, "ipc", driver);
            convert(&native, &normalized, driver, "ipc");
            let before = native_batch(&native, &normalized, driver);
            if !(*driver == "xls" && values.data_type() == &DataType::Int64) {
                assert_eq!(before.column(1), values, "{driver}: first round trip");
            }
            save(&normalized, &before);
            let second = dir.path().join(format!("second.{extension}"));
            let output = dir.path().join("output.arrow");
            let written = convert(&normalized, &second, "ipc", driver);
            let read = convert(&second, &output, driver, "ipc");
            assert_eq!(
                before,
                native_batch(&second, &output, driver),
                "{driver} {:?}",
                values.data_type()
            );
            if matches!(*driver, "csv" | "xls") {
                assert_eq!(
                    written["result"]["write_loss"]["counts"],
                    serde_json::json!([
                        {"categoria":"arrow_metadata_not_preserved", "conteggio":4},
                        {"categoria":"crs_id_not_preserved_absent", "conteggio":1},
                    ]),
                    "{driver}: {written}"
                );
                assert_eq!(
                    read["result"]["read_loss"]["counts"],
                    serde_json::json!([
                        {"categoria":"native_field_order_changed", "conteggio":1},
                    ]),
                    "{driver}: {read}"
                );
            } else {
                empty_report(&written, "write_loss", driver);
                empty_report(&read, "read_loss", driver);
            }
        }
    }
}

#[test]
fn native_kml_defaults_are_compared_with_delivered_geometry() {
    for token in ["0", "false", "1", "true"] {
        for member in ["extrude", "tessellate"] {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("native.kml");
            let target = dir.path().join("output.arrow");
            std::fs::write(&source, format!("<kml><Placemark><LineString><{member}>{token}</{member}><coordinates>1,2 3,4</coordinates></LineString></Placemark></kml>")).unwrap();
            let report = convert(&source, &target, "kml", "ipc");
            let expected = geometry_batch(
                WkbValue::LineString(vec![coordinate(1.0, 2.0), coordinate(3.0, 4.0)]),
                GeometryType::LineString,
            );
            assert_eq!(load(&target).column(0), expected.column(0));
            let changed = matches!(token, "1" | "true");
            let expected = if changed {
                serde_json::json!([
                    {"categoria":"native_foreign_members_not_preserved", "conteggio":1}
                ])
            } else {
                serde_json::json!([])
            };
            assert_eq!(report["result"]["read_loss"]["counts"], expected);
        }
    }
}

#[test]
fn native_geojson_point_xy_has_no_invented_dimension_loss() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("native.geojson");
    let target = dir.path().join("output.arrow");
    std::fs::write(&source, r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Point","coordinates":[1,2]},"properties":{}}]}"#).unwrap();
    let report = convert(&source, &target, "geojson", "ipc");
    let expected = geometry_batch(WkbValue::Point(coordinate(1.0, 2.0)), GeometryType::Point);
    assert_eq!(load(&target).column(0), expected.column(0));
    empty_report(&report, "read_loss", "GeoJSON Point (1,2)");
}
