//! Remaining group A native witnesses, authored without product writers.
use super::*;
use std::io::Write;

pub(super) fn document(driver: &str, element: E) -> Option<Document> {
    if driver == "kml" {
        let mut schema = String::new();
        let (extra, omitted) = match element {
            E::Boolean | E::SignedInteger | E::UnsignedInteger | E::Floating => {
                let (kind, value) = match element {
                    E::Boolean => ("bool", "true"),
                    E::SignedInteger => ("int", "-7"),
                    E::UnsignedInteger => ("uint", "4294967295"),
                    _ => ("double", "1.25"),
                };
                schema = format!("<Schema id=\"s\"><SimpleField name=\"value\" type=\"{kind}\"/></Schema>");
                (format!("<ExtendedData><SchemaData schemaUrl=\"#s\"><SimpleData name=\"value\">{value}</SimpleData></SchemaData></ExtendedData>"), vec![E::ExtendedData, E::ExtendedData])
            }
            E::StructuredValue | E::NestedDictionary => ("<ExtendedData><q:items xmlns:q=\"urn:qualification\"><q:item>7</q:item><q:item>9</q:item></q:items></ExtendedData>".into(), vec![E::ExtendedData]),
            E::Temporal => ("<TimeStamp><when>1970-01-02T00:00:00Z</when></TimeStamp>".into(), vec![E::FeatureMetadata]),
            E::MissingValue => ("<name/>".into(), vec![E::MissingValue]),
            E::FieldOrder => ("<name>first</name><description>second</description>".into(), vec![]),
            E::UnknownDimensions | E::NumberPrecision => (String::new(), vec![]),
            _ => return None,
        };
        return Some(Document {
            bytes: format!("<kml><Document>{schema}<Placemark>{extra}<Point><coordinates>1,2</coordinates></Point></Placemark></Document></kml>").into_bytes(),
            faithful: if element == E::FieldOrder {
                let base = native_base("kml");
                let base = with_value(&base, "name", Arc::new(StringArray::from(vec!["first"])));
                with_value(&base, "description", Arc::new(StringArray::from(vec!["second"])))
            } else { kml_base() },
            omitted,
        });
    }
    if driver == "xls" && matches!(element, E::NumericFormat | E::Temporal) {
        let date = element == E::Temporal;
        let faithful = with_value(
            &native_base(driver),
            "value",
            if date {
                Arc::new(Date32Array::from(vec![1])) as ArrayRef
            } else {
                Arc::new(Float64Array::from(vec![42.0])) as ArrayRef
            },
        );
        let cells = format!("<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>geometry</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>value</t></is></c></row><row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>POINT (1 2)</t></is></c><c r=\"B2\" s=\"1\"><v>{}</v></c></row>",if date {"25570"} else {"42"});
        let source = group_a_xlsx_document(&cells);
        let mut input = zip::ZipArchive::new(std::io::Cursor::new(source)).unwrap();
        let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for index in 0..input.len() {
            output
                .raw_copy_file(input.by_index(index).unwrap())
                .unwrap();
        }
        output
            .start_file("xl/styles.xml", zip::write::SimpleFileOptions::default())
            .unwrap();
        write!(output,"<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><cellXfs count=\"2\"><xf numFmtId=\"0\"/><xf numFmtId=\"{}\"/></cellXfs></styleSheet>", if date {"14"} else {"2"}).unwrap();
        return Some(Document {
            bytes: output.finish().unwrap().into_inner(),
            faithful,
            omitted: vec![E::NumericFormat],
        });
    }
    None
}

pub(super) fn coordinates(driver: &str, root: &Path, row: &mut Value) -> bool {
    if !matches!(driver, "csv" | "xls") {
        return false;
    }
    let bytes = if driver == "csv" {
        b"x,y\n1,2\n".to_vec()
    } else {
        group_a_xlsx_document("<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>x</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>y</t></is></c></row><row r=\"2\"><c r=\"A2\"><v>1</v></c><c r=\"B2\"><v>2</v></c></row>")
    };
    let dir = tempfile::tempdir_in(root).unwrap();
    let input = dir.path().join(format!("xy.{}", extension(driver)));
    let output = dir.path().join("result.arrow");
    std::fs::write(&input, bytes).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_plenora-io"))
        .arg("convert")
        .arg(&input)
        .arg(&output)
        .args([
            "--from",
            driver,
            "--to",
            "ipc",
            "--assume-crs",
            "OGC:CRS84",
            "--in-opt",
            "x_column=x",
            "--in-opt",
            "y_column=y",
        ])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    let mut failures = Vec::new();
    if result.status.success() {
        let reader: Box<dyn plenora_io_core::FormatDriver> = if driver == "csv" {
            Box::new(driver_csv::CsvDriver)
        } else {
            Box::new(driver_xls::XlsDriver)
        };
        let options = plenora_io_core::ReadOptions::from_read_parts(
            plenora_io_model::budget::PipelineBudget::builder()
                .build()
                .unwrap()
                .into_read_parts(),
        )
        .with_assume_crs("OGC:CRS84")
        .with_format_options(
            [
                ("x_column".to_owned(), "x".to_owned()),
                ("y_column".to_owned(), "y".to_owned()),
            ]
            .into(),
        );
        let dataset = reader
            .open(plenora_io_core::Source::Path(input), options)
            .unwrap();
        let after = RecordBatch::try_new(
            dataset.layers()[0].contract.schema.clone(),
            super::super::super::load(&output).columns().to_vec(),
        )
        .unwrap();
        let key = format!(
            "plenora.geometry.native.{}.geometry_encoding",
            if driver == "csv" { "csv" } else { "xlsx" }
        );
        let before = metadata(
            &metadata(&native_base(driver), &key, "xy_columns", false),
            "plenora.geometry.axis_order",
            "lon_lat",
            false,
        );
        let mut differences = compare(&before, &after);
        // The native document has two named coordinate fields. Their values
        // survive in WKB (compared above), but their field identities do not.
        for name in ["x", "y"] {
            if after.schema().field_with_name(name).is_err() {
                differences.push(Difference {
                    element: E::CoordinateColumns,
                    detail: format!("native coordinate field {name} reconstructed as geometry"),
                });
            }
        }
        let native = if driver == "csv" {
            NativeDriver::Csv
        } else {
            NativeDriver::Xls
        };
        let observed = counts(native, D::Read, &differences);
        let declared = report_categories(&report, "read_loss");
        if observed != declared {
            failures.push(json!({"observed":observed,"declared":declared,"differences":format!("{differences:?}"),"schema":format!("{:?}",after.schema())}));
        }
    } else {
        failures.push(json!({"read_failed":report}));
    }
    finish(row, failures);
    row["witness"] = json!("native numeric X/Y columns with explicit coordinate selection");
    true
}
