//! Regressioni comuni dei sei writer testuali, incluso XLSX senza modificarne il driver.
use arrow_array::{
    ArrayRef, BinaryArray, Date32Array, Date64Array, RecordBatch, Time32SecondArray,
    Time64NanosecondArray, TimestampSecondArray,
};
use arrow_schema::{DataType, Field, Schema};
use std::{collections::HashMap, io::Read, path::Path, process::Command, sync::Arc};

fn dataset(path: &Path, column: ArrayRef) {
    let metadata: HashMap<String, String> = [
        ("ARROW:extension:name", "geoarrow.wkb"),
        ("plenora.field_id", "2"),
        ("plenora.geometry.encoding", "wkb"),
        ("plenora.geometry.dimensions", "xy"),
        ("plenora.geometry.spatial_semantics", "geometry"),
        ("plenora.geometry.precision", "float64"),
        ("plenora.geometry.types_declaration", "exact"),
        ("plenora.geometry.types", "point"),
        ("plenora.geometry.crs_resolution", "resolved"),
        ("plenora.geometry.crs_id", "OGC:CRS84"),
        ("plenora.geometry.axis_order", "lon_lat"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    // DXF converte in testo la colonna layer: un attributo arbitrario non raggiungerebbe il convertitore.
    let schema = Arc::new(Schema::new_with_metadata(
        vec![
            Field::new("layer", column.data_type().clone(), true)
                .with_metadata(HashMap::from([("plenora.field_id".into(), "1".into())])),
            Field::new("geom", DataType::Binary, false).with_metadata(metadata),
        ],
        HashMap::from([("plenora.contract.version".into(), "1".into())]),
    ));
    let mut point = vec![1, 1, 0, 0, 0];
    point.extend(12_f64.to_le_bytes());
    point.extend(45_f64.to_le_bytes());
    let geometry: ArrayRef = Arc::new(BinaryArray::from(vec![point.as_slice()]));
    let batch = RecordBatch::try_new(schema.clone(), vec![column, geometry]).unwrap();
    let mut writer =
        arrow_ipc::writer::FileWriter::try_new(std::fs::File::create(path).unwrap(), &schema)
            .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
}

fn convert(input: &Path, output: &Path, format: &str) -> (bool, String) {
    let result = Command::new(env!("CARGO_BIN_EXE_plenora-io"))
        .args([
            "convert",
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "--from",
            "ipc",
            "--to",
            if format == "xlsx" { "xls" } else { format },
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    (
        result.status.success(),
        String::from_utf8(result.stdout).unwrap(),
    )
}

fn output_text(path: &Path, format: &str) -> Vec<u8> {
    if format == "xlsx" {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut text = Vec::new();
        zip.by_name("xl/sharedStrings.xml")
            .unwrap()
            .read_to_end(&mut text)
            .unwrap();
        text
    } else {
        std::fs::read(if format == "shp" {
            path.join("data.dbf")
        } else {
            path.to_owned()
        })
        .unwrap()
    }
}

#[test]
fn tutti_i_writer_rifiutano_temporali_non_rappresentabili() {
    let invalid: Vec<ArrayRef> = vec![
        Arc::new(Date64Array::from(vec![1])),
        Arc::new(Time32SecondArray::from(vec![86_400])),
        Arc::new(Time64NanosecondArray::from(vec![-1])),
    ];
    for format in ["csv", "xlsx", "geojson", "kml", "dxf", "shp"] {
        for column in &invalid {
            let dir = tempfile::tempdir().unwrap();
            let input = dir.path().join("input.arrow");
            let output = dir.path().join(format!(
                "output.{}",
                if format == "shp" { "shp.d" } else { format }
            ));
            dataset(&input, column.clone());
            let (ok, message) = convert(&input, &output, format);
            assert!(!ok, "{format} {:?}: {message}", column.data_type());
            assert!(
                message.contains("temporale")
                    || message.contains("layer_not_representable")
                    || message.contains("cell_not_representable"),
                "{format}: {message}"
            );
            assert!(!output.exists(), "{format}: nessuna pubblicazione");
        }
    }
}

#[test]
fn tutti_i_writer_conservano_limiti_e_timestamp_utc() {
    let valid: Vec<(ArrayRef, &str)> = vec![
        (
            Arc::new(Time64NanosecondArray::from(vec![86_399_999_999_999])),
            "23:59:59.999999999",
        ),
        (
            Arc::new(Date32Array::from(vec![i32::MIN])),
            "-5877641-06-23",
        ),
        (Arc::new(Date32Array::from(vec![i32::MAX])), "5881580-07-11"),
        (
            Arc::new(TimestampSecondArray::from(vec![0]).with_timezone("Europe/Rome")),
            "1970-01-01T00:00:00Z",
        ),
    ];
    for format in ["csv", "xlsx", "geojson", "kml", "dxf", "shp"] {
        for (column, expected) in &valid {
            let dir = tempfile::tempdir().unwrap();
            let input = dir.path().join("input.arrow");
            let output = dir.path().join(format!(
                "output.{}",
                if format == "shp" { "shp.d" } else { format }
            ));
            dataset(&input, column.clone());
            let (ok, message) = convert(&input, &output, format);
            assert!(ok, "{format} {expected}: {message}");
            let bytes = output_text(&output, format);
            assert!(
                bytes
                    .windows(expected.len())
                    .any(|w| w == expected.as_bytes()),
                "{format}: atteso {expected}"
            );
        }
    }
}
