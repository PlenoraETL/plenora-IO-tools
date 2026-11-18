//! Additional native semantic factories. Expected values are authored before IO.
use super::*;

pub(super) fn normalize_native_order(
    differences: &[Difference],
    observed: &mut BTreeMap<String, u64>,
) {
    // Read reports count one changed native ordering, not each displaced field.
    if differences.iter().any(|d| d.element == E::FieldOrder) {
        observed.insert("native_field_order_changed".to_owned(), 1);
    }
}

pub(super) fn document(driver: &str, element: E) -> Option<Document> {
    match driver {
        "geojson" => geojson(element),
        "kml" => kml(element),
        "csv" | "xls" => tabular(driver, element),
        _ => None,
    }
}

fn geojson(element: E) -> Option<Document> {
    let mut faithful = native_base("geojson");
    let mut feature =
        json!({"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[1,2]}});
    let mut omitted = Vec::new();
    match element {
        E::Bbox => {
            // Deliberately differs from the envelope recoverable from the point.
            feature["bbox"] = json!([0, 0, 3, 3]);
            omitted.push(E::Bbox);
        }
        E::StructuredValue | E::NestedDictionary => {
            feature["properties"]["value"] = json!({"items":[1,2]});
            faithful = with_value(
                &faithful,
                "value",
                Arc::new(StringArray::from(vec![r#"{"items":[1,2]}"#])),
            );
            // The payload text survives exactly, but the native object type does not.
            omitted.push(E::StructuredValue);
        }
        E::MissingValue => {
            feature.as_object_mut().unwrap().remove("properties");
            omitted.push(E::MissingValue);
        }
        E::FieldOrder => {
            // Keep the physical member order in the serialized source below.
            faithful = with_value(&faithful, "z", Arc::new(Int64Array::from(vec![7])));
            faithful = with_value(&faithful, "a", Arc::new(Int64Array::from(vec![9])));
            return Some(Document {
                bytes: br#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"z":7,"a":9},"geometry":{"type":"Point","coordinates":[1,2]}}]}"#.to_vec(),
                faithful,
                omitted,
            });
        }
        _ => return None,
    }
    Some(Document {
        bytes: serde_json::to_vec(&json!({"type":"FeatureCollection","features":[feature]}))
            .unwrap(),
        faithful,
        omitted,
    })
}

fn kml(element: E) -> Option<Document> {
    let mut faithful = kml_base();
    let mut omitted = Vec::new();
    let (document, extra, mode) = match element {
        E::ExtendedData => {
            omitted.push(E::ExtendedData);
            ("", "<ExtendedData><Data name=\"value\"><value>native payload</value></Data></ExtendedData>", "absolute")
        }
        E::ExternalReference => {
            omitted.push(E::ExternalReference);
            (
                "<NetworkLink><Link><href>relative/external.kml</href></Link></NetworkLink>",
                "",
                "absolute",
            )
        }
        E::AltitudeMode => {
            omitted.push(E::AltitudeMode);
            let base = generate(E::Z).unwrap().remove(0);
            let base = metadata(&base, "crs", "OGC:CRS84", false);
            faithful = with_value(
                &base,
                "name",
                Arc::new(StringArray::from(vec![None::<&str>])),
            );
            faithful = with_value(
                &faithful,
                "description",
                Arc::new(StringArray::from(vec![None::<&str>])),
            );
            ("", "", "relativeToGround")
        }
        E::FieldNames | E::Description => {
            faithful = with_value(
                &native_base("kml"),
                "name",
                Arc::new(StringArray::from(vec![None::<&str>])),
            );
            faithful = with_value(
                &faithful,
                "description",
                Arc::new(StringArray::from(vec!["native description <&>"])),
            );
            (
                "",
                "<description>native description &lt;&amp;&gt;</description>",
                "absolute",
            )
        }
        _ => return None,
    };
    let coordinates = if element == E::AltitudeMode {
        "1,2,3"
    } else {
        "1,2"
    };
    Some(Document {
        bytes: format!("<kml><Document>{document}<Placemark>{extra}<Point><altitudeMode>{mode}</altitudeMode><coordinates>{coordinates}</coordinates></Point></Placemark></Document></kml>").into_bytes(),
        faithful,
        omitted,
    })
}

fn tabular(driver: &str, element: E) -> Option<Document> {
    if element == E::UnknownDimensions {
        return Some(Document {
            bytes: if driver == "csv" {
                b"geometry\n\"POINT (1 2)\"\n".to_vec()
            } else {
                group_a_xlsx_document("<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>geometry</t></is></c></row><row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>POINT (1 2)</t></is></c></row>")
            },
            faithful: native_base(driver),
            omitted: Vec::new(),
        });
    }
    if element != E::FieldOrder {
        return None;
    }
    let faithful = with_value(
        &native_base(driver),
        "value",
        Arc::new(StringArray::from(vec!["payload"])),
    )
    .project(&[1, 0])
    .unwrap();
    let bytes = if driver == "csv" {
        b"value,geometry\npayload,\"POINT (1 2)\"\n".to_vec()
    } else {
        group_a_xlsx_document("<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>value</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>geometry</t></is></c></row><row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>payload</t></is></c><c r=\"B2\" t=\"inlineStr\"><is><t>POINT (1 2)</t></is></c></row>")
    };
    Some(Document {
        bytes,
        faithful,
        omitted: Vec::new(),
    })
}

pub(super) fn rejection(driver: &str, element: E, root: &Path, row: &mut Value) -> bool {
    let bytes = match (driver, element) {
        ("geojson", E::EmptyGeometry) => br#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":{"type":"GeometryCollection","geometries":[]}}]}"#.to_vec(),
        ("kml", E::EmptyGeometry) => b"<kml><Document><Placemark><MultiGeometry></MultiGeometry></Placemark></Document></kml>".to_vec(),
        ("csv", E::EmptyPoint) => b"geometry\n\"POINT EMPTY\"\n".to_vec(),
        ("xls", E::EmptyPoint) => group_a_xlsx_document("<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>geometry</t></is></c></row><row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>POINT EMPTY</t></is></c></row>"),
        _ => return false,
    };
    let dir = tempfile::tempdir_in(root).unwrap();
    let input = dir.path().join(format!("source.{}", extension(driver)));
    let output = dir.path().join("result.arrow");
    std::fs::write(&input, &bytes).unwrap();
    let (ok, report) = if driver == "kml" {
        let result = Command::new(env!("CARGO_BIN_EXE_plenora-io"))
            .arg("convert")
            .arg(&input)
            .arg(&output)
            .args(["--from", "kml", "--to", "ipc", "--layer", "2"])
            .output()
            .unwrap();
        (
            result.status.success(),
            serde_json::from_slice(&result.stdout).unwrap(),
        )
    } else {
        run(&input, &output, driver, "ipc")
    };
    let expected = plenora_io_model::error_table::Situation::InvalidContent.rule();
    let mut failures = Vec::new();
    if ok
        || report["error"]["category"] != json!(expected.category)
        || report["error"]["code"] != expected.wire_code
        || output.exists()
    {
        failures.push(json!({"expected_rejection":"InvalidContent","report":report,"output_exists":output.exists()}));
    }
    finish(row, failures);
    row["witness"] = json!("direct native empty geometry; typed rejection and no output");
    true
}
