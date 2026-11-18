//! Confronto dei valori e dello schema attraverso il confine CLI.
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use arrow_array::{
    ArrayRef, BinaryArray, Float32Array, Int16Array, Int32Array, Int8Array, RecordBatch,
    StringArray, UInt8Array,
};
use arrow_schema::{DataType, Field, Schema};
use serde_json::Value;

fn convert(source: &Path, destination: &Path, from: &str, to: &str, options: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_plenora-io"))
        .args(["convert"])
        .arg(source)
        .arg(destination)
        .args(["--from", from, "--to", to])
        .args(options)
        .output()
        .unwrap();
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{value}");
    value
}

fn save(batch: &RecordBatch, path: &Path) {
    let mut writer =
        arrow_ipc::writer::FileWriter::try_new(File::create(path).unwrap(), &batch.schema())
            .unwrap();
    writer.write(batch).unwrap();
    writer.finish().unwrap();
}

fn load(path: &Path) -> RecordBatch {
    arrow_ipc::reader::FileReader::try_new(File::open(path).unwrap(), None)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
}

fn has_category(result: &Value, category: &str) -> bool {
    result["result"]["write_loss"]["counts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["categoria"] == category && row["conteggio"].as_u64().unwrap() > 0)
}

fn assert_numeric_values(value: &ArrayRef, original: &DataType) {
    match value.data_type() {
        DataType::Int64 => assert_eq!(
            value
                .as_any()
                .downcast_ref::<arrow_array::Int64Array>()
                .unwrap(),
            &arrow_array::Int64Array::from(vec![Some(1), None])
        ),
        DataType::Float64 => {
            let expected = if original == &DataType::Float32 {
                0.5
            } else {
                1.0
            };
            assert_eq!(
                value
                    .as_any()
                    .downcast_ref::<arrow_array::Float64Array>()
                    .unwrap(),
                &arrow_array::Float64Array::from(vec![Some(expected), None])
            );
        }
        other => panic!("tipo di rilettura inatteso: {other:?}"),
    }
}

#[test]
fn catalogo_e_tabella_enumerano_gli_stessi_driver_tipi_e_dimensioni() {
    let catalog = plenora_io_tools::cmd_catalog();
    let drivers = catalog["drivers"].as_array().unwrap();
    assert_eq!(drivers.len(), plenora_io_core::loss_table::DRIVERS.len());
    for driver in drivers {
        let row = plenora_io_core::loss_table::rules(driver["id"].as_str().unwrap()).unwrap();
        let serialized = serde_json::to_value(row.allowed_types).unwrap();
        assert_eq!(driver["write_capabilities"]["allowed_types"], serialized);
        assert_eq!(
            driver["write_capabilities"]["geometry"]["dimensions"],
            serde_json::to_value(row.dimensions).unwrap()
        );
    }
}

#[test]
fn csv_xlsx_tabellari_conservano_null_stringa_vuota_e_spazi() {
    let dir = tempfile::tempdir().unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new("text", DataType::Utf8, true)]));
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(StringArray::from(vec![
            Some(""),
            None,
            Some(" "),
            Some("z"),
        ]))],
    )
    .unwrap();
    let source = dir.path().join("source.arrow");
    save(&batch, &source);
    for (driver, extension) in [("csv", "csv"), ("xls", "xlsx")] {
        let native = dir.path().join(format!("native.{extension}"));
        let back = dir.path().join(format!("{driver}.arrow"));
        convert(&source, &native, "ipc", driver, &[]);
        convert(&native, &back, driver, "ipc", &[]);
        let actual = load(&back);
        assert_eq!(actual.num_columns(), 1);
        assert_eq!(actual.column(0), batch.column(0));
        assert!(!plenora_io_model::geometry::is_geometry_field(
            actual.schema().field(0)
        ));
    }
}

#[test]
fn numeri_stretti_e_metadati_persi_hanno_categoria_e_indice() {
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(Int8Array::from(vec![Some(1), None])),
        Arc::new(Int16Array::from(vec![Some(1), None])),
        Arc::new(Int32Array::from(vec![Some(1), None])),
        Arc::new(UInt8Array::from(vec![Some(1), None])),
        Arc::new(Float32Array::from(vec![Some(0.5), None])),
    ];
    let mut geometry = plenora_io_model::contract::GeometryColumnContract::wkb_xy(
        plenora_io_model::contract::FieldId(0),
        "geometry",
        plenora_io_model::crs::ResolvedCrs::wgs84(),
        true,
    );
    geometry.set_exact_geometry_types(vec![plenora_io_model::contract::GeometryType::Point]);
    let geom_field = plenora_io_model::geometry::with_geometry_contract_metadata(
        &Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([(
            "ARROW:extension:name".into(),
            "geoarrow.wkb".into(),
        )])),
        &geometry,
    );
    let point = [
        vec![1, 1, 0, 0, 0],
        1_f64.to_le_bytes().to_vec(),
        2_f64.to_le_bytes().to_vec(),
    ]
    .concat();
    for array in arrays {
        let dir = tempfile::tempdir().unwrap();
        let field = Field::new("value", array.data_type().clone(), true)
            .with_metadata(HashMap::from([("opaque_field".into(), "secret".into())]));
        let schema = Arc::new(Schema::new_with_metadata(
            vec![geom_field.clone(), field],
            HashMap::from([
                ("opaque_schema".into(), "secret".into()),
                ("plenora.contract.version".into(), "1".into()),
            ]),
        ));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(BinaryArray::from(vec![
                    Some(point.as_slice()),
                    Some(point.as_slice()),
                ])),
                array,
            ],
        )
        .unwrap();
        let source = dir.path().join("source.arrow");
        save(&batch, &source);
        for (driver, extension) in [("geojson", "geojson"), ("shp", "shp.d"), ("xls", "xlsx")] {
            let native = dir.path().join(format!("native.{extension}"));
            let back = dir.path().join(format!("{driver}.arrow"));
            let result = convert(&source, &native, "ipc", driver, &[]);
            assert!(has_category(&result, "coercion tipo attributo"));
            assert!(has_category(&result, "arrow_metadata_not_preserved"));
            let examples = result["result"]["write_loss"]["esempi"].as_array().unwrap();
            assert!(examples
                .iter()
                .any(|e| e["category"] == "coercion tipo attributo" && e["field_index"] == 1));
            assert!(!result["result"]["write_loss"]
                .to_string()
                .contains("secret"));
            let options = if driver == "xls" {
                &[
                    "--in-opt",
                    "wkt_column=geometry",
                    "--assume-crs",
                    "OGC:CRS84",
                ][..]
            } else {
                &[]
            };
            convert(&native, &back, driver, "ipc", options);
            let actual = load(&back);
            let value = actual.column(actual.schema().index_of("value").unwrap());
            assert_ne!(value.data_type(), batch.column(1).data_type());
            assert_eq!(value.null_count(), 1);
            assert_eq!(value.len(), 2);
            assert_numeric_values(value, batch.column(1).data_type());
        }
    }
}
