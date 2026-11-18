//! Native schema and value regressions through the public CLI.
use arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Date32Array, Float32Array, Float64Array, Int64Array,
    Int8Array, RecordBatch, StringArray, UInt64Array,
};
use arrow_schema::{DataType, Field, Schema};
use plenora_io_model::{
    contract::{FieldId, GeometryColumnContract, GeometryType},
    crs::ResolvedCrs,
};
use serde_json::Value;
use std::{collections::HashMap, fs::File, path::Path, process::Command, sync::Arc};

fn convert(src: &Path, dst: &Path, from: &str, to: &str) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_plenora-io"))
        .arg("convert")
        .arg(src)
        .arg(dst)
        .args(["--from", from, "--to", to, "--layer", "0"])
        .output()
        .unwrap();
    assert!(
        output.stderr.is_empty(),
        "{from} -> {to}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{from} -> {to}: {result}");
    result
}
fn count(result: &Value, direction: &str, category: &str) -> u64 {
    result["result"][direction]["counts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["categoria"] == category)
        .map(|c| c["conteggio"].as_u64().unwrap())
        .sum()
}
fn load(path: &Path) -> RecordBatch {
    arrow_ipc::reader::FileReader::try_new(File::open(path).unwrap(), None)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
}

#[test]
fn shp_dxf_filegdb_native_roundtrip_declares_schema_and_coordinate_changes() {
    let mut drivers = vec![("shp", "shp.d"), ("dxf", "dxf")];
    if cfg!(feature = "gdal-backend") {
        drivers.push(("filegdb", "gdb"));
    }
    for (driver, extension) in drivers {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.arrow");
        let native = dir.path().join(format!("native.{extension}"));
        let back = dir.path().join("back.arrow");
        let mut geometry =
            GeometryColumnContract::wkb_xy(FieldId(1), "geom", ResolvedCrs::wgs84(), false);
        geometry.set_exact_geometry_types(vec![GeometryType::Point]);
        let field = plenora_io_model::geometry::with_geometry_contract_metadata(
            &Field::new("geom", DataType::Binary, false).with_metadata(HashMap::from([(
                "ARROW:extension:name".into(),
                "geoarrow.wkb".into(),
            )])),
            &geometry,
        );
        let point = [
            vec![1, 1, 0, 0, 0],
            1.123_456_789_012_345_f64.to_le_bytes().to_vec(),
            1e-20_f64.to_le_bytes().to_vec(),
        ]
        .concat();
        let schema =
            plenora_io_model::geometry::with_contract_version(Arc::new(Schema::new(vec![
                Field::new("secret", DataType::Utf8, false),
                field,
            ])));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec!["payload"])),
                Arc::new(BinaryArray::from(vec![point.as_slice()])),
            ],
        )
        .unwrap();
        let mut writer =
            arrow_ipc::writer::FileWriter::try_new(File::create(&source).unwrap(), &schema)
                .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
        drop(writer);
        let written = convert(&source, &native, "ipc", driver);
        assert_eq!(
            count(&written, "write_loss", "native_field_order_changed"),
            if driver == "dxf" { 1 } else { 2 }
        );
        assert_eq!(
            count(&written, "write_loss", "native_not_null_not_preserved"),
            if driver == "dxf" { 1 } else { 2 }
        );
        assert!(!written["result"]["write_loss"]
            .to_string()
            .contains("secret"));
        assert!(!written["result"]["write_loss"]
            .to_string()
            .contains("payload"));
        let read = convert(&native, &back, driver, "ipc");
        let actual = load(&back);
        assert_eq!(actual.num_rows(), 1);
        assert_eq!(actual.schema().field(0).name(), "geometry");
        if driver == "dxf" {
            assert_eq!(
                count(&written, "write_loss", "dxf_attribute_not_represented"),
                1
            );
            assert!(actual.schema().index_of("secret").is_err());
        } else {
            assert_eq!(actual.column(1), batch.column(0));
            assert_eq!(
                count(&read, "read_loss", "native_field_order_changed"),
                u64::from(driver != "shp")
            );
        }
        if driver == "shp" {
            assert_eq!(actual.column(0), batch.column(1));
        } else {
            assert_ne!(actual.column(0), batch.column(1));
            assert_eq!(
                count(&written, "write_loss", "native_coordinate_precision"),
                1
            );
        }
    }
}

// La matrice tiene adiacenti fixture, esito e confronto dei valori per ogni driver.
#[test]
#[allow(clippy::too_many_lines)]
fn native_geometry_promotions_rings_empty_and_signed_zero() {
    use arrow_array::Array;
    use plenora_io_model::contract::CoordinateDimensions;
    use plenora_io_model::wkb::{encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};
    let coord = |x, y| WkbCoordinate {
        x,
        y,
        z: None,
        m: None,
    };
    let ring = vec![
        coord(0.0, 0.0),
        coord(10.0, 0.0),
        coord(10.0, 10.0),
        coord(0.0, 10.0),
        coord(0.0, 0.0),
    ];
    let cases = [
        (
            GeometryType::LineString,
            WkbValue::LineString(vec![coord(1e-20, -0.0), coord(1.123_456_789_012_345, 2.0)]),
        ),
        (GeometryType::Polygon, WkbValue::Polygon(vec![ring])),
        (
            GeometryType::Point,
            WkbValue::Point(coord(f64::NAN, f64::NAN)),
        ),
        (GeometryType::MultiPolygon, WkbValue::MultiPolygon(vec![])),
    ];
    for (kind, value) in cases {
        let empty = matches!(kind, GeometryType::Point | GeometryType::MultiPolygon);
        let mut drivers = vec![("shp", "shp.d"), ("dxf", "dxf")];
        if cfg!(feature = "gdal-backend") {
            drivers.push(("filegdb", "gdb"));
        }
        for (driver, extension) in drivers {
            let (native_kind, native_value) = if driver == "filegdb" && !empty {
                let child = WkbGeometry {
                    value: value.clone(),
                    dimensions: CoordinateDimensions::Xy,
                    srid: None,
                };
                if kind == GeometryType::Polygon {
                    (
                        GeometryType::MultiPolygon,
                        WkbValue::MultiPolygon(vec![child]),
                    )
                } else {
                    (
                        GeometryType::MultiLineString,
                        WkbValue::MultiLineString(vec![child]),
                    )
                }
            } else {
                (kind, value.clone())
            };
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("source.arrow");
            let native = dir.path().join(format!("native.{extension}"));
            let back = dir.path().join("back.arrow");
            let mut geometry =
                GeometryColumnContract::wkb_xy(FieldId(0), "geometry", ResolvedCrs::wgs84(), true);
            geometry.set_exact_geometry_types(vec![native_kind]);
            let field = plenora_io_model::geometry::with_geometry_contract_metadata(
                &Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([(
                    "ARROW:extension:name".into(),
                    "geoarrow.wkb".into(),
                )])),
                &geometry,
            );
            let schema =
                plenora_io_model::geometry::with_contract_version(Arc::new(Schema::new(vec![
                    field,
                ])));
            let bytes = encode_wkb(
                &WkbGeometry {
                    value: native_value,
                    dimensions: CoordinateDimensions::Xy,
                    srid: None,
                },
                WkbFlavor::Iso,
            )
            .unwrap();
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![bytes.as_slice()]))],
            )
            .unwrap();
            let mut writer =
                arrow_ipc::writer::FileWriter::try_new(File::create(&source).unwrap(), &schema)
                    .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();
            drop(writer);
            if empty && driver != "filegdb" && !(driver == "shp" && kind == GeometryType::Point) {
                let output = Command::new(env!("CARGO_BIN_EXE_plenora-io"))
                    .arg("convert")
                    .arg(&source)
                    .arg(&native)
                    .args(["--from", "ipc", "--to", driver])
                    .output()
                    .unwrap();
                assert!(
                    !output.status.success(),
                    "driver={driver}, kind={kind:?}, stdout={}",
                    String::from_utf8_lossy(&output.stdout)
                );
                assert!(output.stderr.is_empty());
                assert!(!native.exists());
                continue;
            }
            let written = convert(&source, &native, "ipc", driver);
            convert(&native, &back, driver, "ipc");
            let actual = load(&back);
            assert_eq!(actual.num_rows(), 1);
            if empty && driver == "shp" {
                assert_eq!(actual.column(0), batch.column(0));
            } else if empty {
                assert!(actual.column(0).is_null(0));
                assert_eq!(count(&written, "write_loss", "native_missing_as_null"), 1);
            } else {
                if driver == "shp" {
                    assert_eq!(
                        count(&written, "write_loss", "native_geometry_type_changed"),
                        1
                    );
                    let expected = if kind == GeometryType::Polygon {
                        "multipolygon"
                    } else {
                        "multilinestring"
                    };
                    assert_eq!(
                        actual.schema().field(0).metadata()["plenora.geometry.types"],
                        expected
                    );
                } else {
                    assert_eq!(
                        count(&written, "write_loss", "native_coordinate_precision"),
                        u64::from(kind == GeometryType::LineString)
                    );
                }
                if kind == GeometryType::Polygon && driver != "dxf" {
                    assert_eq!(
                        count(&written, "write_loss", "native_ring_orientation_changed"),
                        1
                    );
                    assert_ne!(actual.column(0), batch.column(0));
                }
                if kind == GeometryType::LineString && driver != "shp" {
                    let binary = actual
                        .column(0)
                        .as_any()
                        .downcast_ref::<BinaryArray>()
                        .unwrap();
                    let got = binary.value(0);
                    let offset = if driver == "filegdb" { 18 } else { 9 };
                    let x = f64::from_le_bytes(got[offset..offset + 8].try_into().unwrap());
                    let y = f64::from_le_bytes(got[offset + 8..offset + 16].try_into().unwrap());
                    assert_eq!(x.to_bits(), 0.0_f64.to_bits());
                    assert_eq!(y.to_bits(), 0.0_f64.to_bits());
                }
            }
        }
    }
}

#[test]
fn native_schema_values_and_losses_roundtrip() {
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(Int8Array::from(vec![Some(-128), None])),
        Arc::new(Int64Array::from(vec![Some(i64::MAX), None])),
        Arc::new(UInt64Array::from(vec![Some(u64::MAX), None])),
        Arc::new(Float32Array::from(vec![Some(0.1), None])),
        Arc::new(Float64Array::from(vec![Some(1e-20), None])),
        Arc::new(BooleanArray::from(vec![Some(true), None])),
        Arc::new(Date32Array::from(vec![Some(0), None])),
        Arc::new(StringArray::from(vec![Some(""), None])),
        Arc::new(BinaryArray::from(vec![Some(&b"abc"[..]), None])),
        Arc::new(
            arrow_array::Decimal128Array::from(vec![Some(123_i128), None])
                .with_precision_and_scale(20, 4)
                .unwrap(),
        ),
        Arc::new(arrow_array::ListArray::from_iter_primitive::<
            arrow_array::types::Int32Type,
            _,
            _,
        >(vec![Some(vec![Some(1)]), None])),
    ];
    for array in arrays {
        let batch = fixture(&array);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.arrow");
        let mut writer =
            arrow_ipc::writer::FileWriter::try_new(File::create(&source).unwrap(), &batch.schema())
                .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
        drop(writer);
        for driver in ["geojson", "kml"] {
            let native = dir.path().join(format!("native.{driver}"));
            let back = dir.path().join(format!("{driver}.arrow"));
            if matches!(
                array.data_type(),
                DataType::Binary | DataType::Decimal128(_, _) | DataType::List(_)
            ) {
                let output = Command::new(env!("CARGO_BIN_EXE_plenora-io"))
                    .arg("convert")
                    .arg(&source)
                    .arg(&native)
                    .args(["--from", "ipc", "--to", driver])
                    .output()
                    .unwrap();
                assert!(!output.status.success());
                assert!(output.stderr.is_empty());
                let result: Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(result["error"]["category"], "unsupported");
                assert!(!native.exists());
                continue;
            }
            check_roundtrip(driver, &source, &native, &back, &batch, &array);
        }
    }
}

fn fixture(array: &ArrayRef) -> RecordBatch {
    let mut geometry =
        GeometryColumnContract::wkb_xy(FieldId(1), "geom", ResolvedCrs::wgs84(), true);
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let geom_field = plenora_io_model::geometry::with_geometry_contract_metadata(
        &Field::new("geom", DataType::Binary, true).with_metadata(HashMap::from([(
            "ARROW:extension:name".into(),
            "geoarrow.wkb".into(),
        )])),
        &geometry,
    );
    let point = [
        vec![1, 1, 0, 0, 0],
        1.123_456_789_012_345_f64.to_le_bytes().to_vec(),
        1e-20_f64.to_le_bytes().to_vec(),
    ]
    .concat();
    let schema = Arc::new(Schema::new_with_metadata(
        vec![
            Field::new("z", array.data_type().clone(), true).with_metadata(HashMap::from([(
                "secret_key".into(),
                "secret_value".into(),
            )])),
            geom_field,
            Field::new("a", DataType::Utf8, false),
            Field::new("name", DataType::Utf8, true),
            Field::new("description", DataType::Utf8, true),
        ],
        HashMap::from([
            ("secret_schema".into(), "secret_value".into()),
            ("plenora.contract.version".into(), "1".into()),
        ]),
    ));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            array.clone(),
            Arc::new(BinaryArray::from(vec![Some(point.as_slice()), None])),
            Arc::new(StringArray::from(vec!["", " "])),
            Arc::new(StringArray::from(vec![Some(""), None])),
            Arc::new(StringArray::from(vec![Some("text"), Some(" ")])),
        ],
    )
    .unwrap();
    batch
}

fn check_roundtrip(
    driver: &str,
    source: &Path,
    native: &Path,
    back: &Path,
    batch: &RecordBatch,
    array: &ArrayRef,
) {
    let write = convert(source, native, "ipc", driver);
    assert_eq!(
        count(&write, "write_loss", "native_not_null_not_preserved"),
        u64::from(driver != "kml")
    );
    assert!(count(&write, "write_loss", "native_field_order_changed") > 0);
    assert_eq!(
        count(&write, "write_loss", "native_fields_reconstructed"),
        1
    );
    assert!(count(&write, "write_loss", "arrow_metadata_not_preserved") > 0);
    assert!(!write["result"]["write_loss"].to_string().contains("secret"));
    if driver != "kml" && plenora_io_core::loss_table::normalizes_type(driver, array.data_type()) {
        assert_eq!(count(&write, "write_loss", "coercion tipo attributo"), 1);
    }
    let read = convert(native, back, driver, "ipc");
    let actual = load(back);
    assert_eq!(actual.column(0), batch.column(1));
    for name in ["name", "description"] {
        assert_eq!(
            actual.column(actual.schema().index_of(name).unwrap()),
            batch.column(batch.schema().index_of(name).unwrap())
        );
    }
    assert!(actual
        .schema()
        .fields()
        .iter()
        .all(|field| field.is_nullable()));
    assert_eq!(
        count(&read, "read_loss", "native_dimensions_not_declared"),
        0
    );
    if driver == "kml" {
        assert_eq!(actual.num_columns(), 3);
        assert_eq!(
            count(&write, "write_loss", "kml_extended_data_not_preserved"),
            2
        );
        assert_eq!(
            count(&read, "read_loss", "kml_extended_data_not_preserved"),
            2
        );
    } else {
        assert_eq!(
            actual
                .schema()
                .fields()
                .iter()
                .map(|f| f.name().as_str())
                .collect::<Vec<_>>(),
            ["geometry", "a", "description", "name", "z"]
        );
        assert_eq!(actual.column(1), batch.column(2));
        let z = actual.column(4);
        assert_eq!(z.null_count(), array.null_count());
        if z.data_type() == array.data_type() {
            assert_eq!(z, array);
        } else {
            assert_eq!(count(&write, "write_loss", "coercion tipo attributo"), 1);
            assert_converted_value(z, array.data_type());
        }
    }
}

fn assert_converted_value(z: &ArrayRef, original: &DataType) {
    match original {
        DataType::Int8 => assert_eq!(
            z.as_any().downcast_ref::<Int64Array>().unwrap().value(0),
            -128
        ),
        DataType::Float32 => assert_eq!(
            z.as_any()
                .downcast_ref::<Float64Array>()
                .unwrap()
                .value(0)
                .to_bits(),
            f64::from(0.1_f32).to_bits()
        ),
        DataType::Date32 => assert_eq!(
            z.as_any().downcast_ref::<StringArray>().unwrap().value(0),
            "1970-01-01"
        ),
        DataType::UInt64 => assert_eq!(
            z.as_any().downcast_ref::<StringArray>().unwrap().value(0),
            u64::MAX.to_string()
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn kml_declares_standard_fields_added_on_readback() {
    let array: ArrayRef = Arc::new(StringArray::from(vec![Some("value"), None]));
    let batch = fixture(&array).project(&[0, 1, 2]).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.arrow");
    let native = dir.path().join("native.kml");
    let back = dir.path().join("back.arrow");
    let mut writer =
        arrow_ipc::writer::FileWriter::try_new(File::create(&source).unwrap(), &batch.schema())
            .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    drop(writer);
    let write = convert(&source, &native, "ipc", "kml");
    assert_eq!(
        count(&write, "write_loss", "native_fields_reconstructed"),
        3
    );
    convert(&native, &back, "kml", "ipc");
    let actual = load(&back);
    assert_eq!(actual.num_columns(), 3);
    assert_eq!(actual.schema().fields()[1].name(), "name");
    assert_eq!(actual.schema().fields()[2].name(), "description");
    assert_eq!(actual.column(1).null_count(), 2);
    assert_eq!(actual.column(2).null_count(), 2);
}

#[test]
#[allow(clippy::too_many_lines)] // Independent source, native output and report oracle for all drivers.
fn metadata_counts_equal_independent_roundtrip_comparison() {
    fn save(batch: &RecordBatch, path: &Path) {
        let mut writer =
            arrow_ipc::writer::FileWriter::try_new(File::create(path).unwrap(), &batch.schema())
                .unwrap();
        writer.write(batch).unwrap();
        writer.finish().unwrap();
    }
    fn back(native: &Path, output: &Path, driver: &str) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_plenora-io"));
        command
            .arg("read")
            .arg(native)
            .args(["--layer", "0", "--output"])
            .arg(output);
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{driver}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
    }
    let mut drivers = vec![
        ("geojson", "json"),
        ("kml", "kml"),
        ("csv", "csv"),
        ("xls", "xlsx"),
        ("shp", "shp.d"),
        ("dxf", "dxf"),
        ("ipc", "arrow"),
        ("geoparquet", "parquet"),
        ("gpkg", "gpkg"),
    ];
    if cfg!(feature = "gdal-backend") {
        drivers.push(("filegdb", "gdb"));
    }
    for (driver, extension) in drivers {
        let dir = tempfile::tempdir().unwrap();
        let mut crs = ResolvedCrs::wgs84();
        if driver == "gpkg" {
            crs.id = Some("EPSG:4326".into());
        }
        let mut geometry = GeometryColumnContract::wkb_xy(FieldId(1), "geometry", crs, true);
        geometry.set_exact_geometry_types(vec![GeometryType::Point]);
        let field = plenora_io_model::geometry::with_geometry_contract_metadata(
            &Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([(
                "ARROW:extension:name".into(),
                "geoarrow.wkb".into(),
            )])),
            &geometry,
        );
        let schema =
            plenora_io_model::geometry::with_contract_version(Arc::new(Schema::new(vec![field])));
        let point = [
            vec![1, 1, 0, 0, 0],
            1.5_f64.to_le_bytes().to_vec(),
            2.5_f64.to_le_bytes().to_vec(),
        ]
        .concat();
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(BinaryArray::from(vec![point.as_slice()]))],
        )
        .unwrap();
        let seed = dir.path().join("seed.arrow");
        save(&batch, &seed);
        let first = dir.path().join(format!("first.{extension}"));
        convert(&seed, &first, "ipc", driver);
        let normalized = dir.path().join("normalized.arrow");
        back(&first, &normalized, driver);
        let batch = load(&normalized);
        for changed in [false, true] {
            let mut metadata = batch.schema().metadata().clone();
            let mut fields: Vec<_> = batch
                .schema()
                .fields()
                .iter()
                .map(|f| f.as_ref().clone())
                .collect();
            if changed {
                metadata.insert("private.schema", "private-value");
                let mut values = fields[0].metadata().clone();
                values.insert("private.field", "private-value");
                fields[0] = fields[0].clone().with_metadata(values);
            }
            let before = Arc::new(Schema::new_with_metadata(fields, metadata));
            let source_batch =
                RecordBatch::try_new(before.clone(), batch.columns().to_vec()).unwrap();
            let source = dir.path().join(format!("source_{changed}.arrow"));
            save(&source_batch, &source);
            let native = dir.path().join(format!("out-{changed}.{extension}"));
            let written = convert(&source, &native, "ipc", driver);
            let output = dir.path().join(format!("back-{changed}.arrow"));
            back(&native, &output, driver);
            let actual = load(&output);
            // Compare the native reader, before IPC publication can reconstruct field IDs.
            let reader_driver: Box<dyn plenora_io_core::FormatDriver> = match driver {
                "geojson" => Box::new(driver_geojson::GeoJsonDriver),
                "kml" => Box::new(driver_kml::KmlDriver),
                "csv" => Box::new(driver_csv::CsvDriver),
                "xls" => Box::new(driver_xls::XlsDriver),
                "shp" => Box::new(driver_shp::ShpDriver),
                "dxf" => Box::new(driver_dxf::DxfDriver),
                "filegdb" => Box::new(driver_filegdb::FileGdbDriver),
                "ipc" => Box::new(driver_ipc::IpcDriver),
                "gpkg" => Box::new(driver_gpkg::GpkgDriver),
                "geoparquet" => Box::new(driver_geoparquet::GeoParquetDriver),
                _ => unreachable!(),
            };
            let dataset = reader_driver
                .open(
                    plenora_io_core::Source::Path(native.clone()),
                    plenora_io_core::ReadOptions::from_read_parts(
                        plenora_io_model::budget::PipelineBudget::builder()
                            .build()
                            .unwrap()
                            .into_read_parts(),
                    ),
                )
                .unwrap();
            let after = &dataset.layers()[0].contract.schema;
            let mut expected = before
                .metadata()
                .iter()
                .filter(|(k, v)| after.metadata().get(k) != Some(*v))
                .count();
            for field in before.fields() {
                let output_field = after.fields().iter().find(|f| f.name() == field.name());
                expected += field
                    .metadata()
                    .iter()
                    .filter(|(k, v)| output_field.and_then(|f| f.metadata().get(k)) != Some(*v))
                    .count();
            }
            assert_eq!(
                count(
                    &written,
                    "write_loss",
                    if driver == "gpkg" {
                        "gpkg_metadata_not_preserved"
                    } else {
                        "arrow_metadata_not_preserved"
                    }
                ),
                expected as u64,
                "{driver} changed={changed}: {written}"
            );
            assert_eq!(actual.column(0), source_batch.column(0), "{driver}");
        }
    }
}
