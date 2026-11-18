//! Additive qualification for group A. Native bytes never use an IO writer.
use super::native::group_a_xlsx_document;
use super::*;
#[path = "group_a_second.rs"]
mod second;

pub(super) fn owns(driver: &str) -> bool {
    matches!(driver, "geojson" | "kml" | "csv" | "xls" | "ipc")
}

fn factory(element: E) -> Option<Vec<RecordBatch>> {
    let base = geometry_batch(WkbValue::Point(coordinate(1.0, 2.0)), GeometryType::Point);
    let batch = match element {
        E::Srid => metadata(&base, "plenora.geometry.srid", "4326", false),
        E::CrsDefinition | E::DefinitionFormat => {
            let b = metadata(
                &base,
                "plenora.geometry.crs_definition",
                r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]]"#,
                false,
            );
            metadata(&b, "plenora.geometry.crs_definition_format", "wkt", false)
        }
        _ => return None,
    };
    Some(vec![batch])
}

pub(super) fn exercise(
    driver: &str,
    native: NativeDriver,
    element: E,
    direction: D,
    root: &Path,
    row: &mut Value,
) -> bool {
    if !owns(driver) {
        return false;
    }
    if direction == D::Read && driver == "ipc" {
        return ipc_read(element, root, row);
    }
    if direction == D::Read && driver != "ipc" {
        if second::rejection(driver, element, root, row) {
            return true;
        }
        return read(driver, native, element, root, row);
    }
    let Some(fixtures) = factory(element) else {
        return false;
    };
    let mut failures = Vec::new();
    for (variant, before) in fixtures.iter().enumerate() {
        let dir = tempfile::tempdir_in(root).unwrap();
        let input = dir.path().join("input.arrow");
        let output = dir.path().join(format!("native.{}", extension(driver)));
        let back = dir.path().join("back.arrow");
        save(&input, before);
        let (ok, written) = run(&input, &output, "ipc", driver);
        if !ok {
            failures.push(json!({"variant":variant,"write_failed":written}));
            continue;
        }
        let (ok, report) = run(&output, &back, driver, "ipc");
        if !ok {
            failures.push(json!({"variant":variant,"read_failed":report}));
            continue;
        }
        let after = without_external_crs(native_batch(&output, &back, driver), driver);
        let differences = compare(before, &after);
        let observed = counts(native, direction, &differences);
        let declared = report_categories(
            if direction == D::Read {
                &report
            } else {
                &written
            },
            if direction == D::Read {
                "read_loss"
            } else {
                "write_loss"
            },
        );
        if observed != declared {
            failures.push(json!({"variant":variant,"observed":observed,"declared":declared,"differences":format!("{differences:?}")}));
        }
    }
    finish(row, failures);
    row["factory"] = json!(format!("group-A semantic {element:?}"));
    true
}

fn counts(native: NativeDriver, direction: D, differences: &[Difference]) -> BTreeMap<String, u64> {
    let mut result = BTreeMap::new();
    for difference in differences {
        *result
            .entry(category(native, direction, difference))
            .or_default() += 1;
        if direction == D::Write
            && difference.element == E::Srid
            && difference.detail == "metadata plenora.geometry.srid"
        {
            *result
                .entry("srid_not_preserved_absent".to_owned())
                .or_default() += 1;
        }
    }
    result
}
fn finish(row: &mut Value, failures: Vec<Value>) {
    row["status"] = json!(if failures.is_empty() {
        "passed"
    } else {
        "failed"
    });
    row["failures"] = Value::Array(failures);
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// Values are native scalar types, not the entire Arrow subtype family.
fn scalar(element: E) -> Option<Vec<(Value, ArrayRef)>> {
    Some(match element {
        E::Utf8 | E::FieldNames => {
            vec![(
                json!("\u{e8}\u{6f22}\u{5b57}<&"),
                Arc::new(StringArray::from(vec!["\u{e8}\u{6f22}\u{5b57}<&"])),
            )]
        }
        E::EmptyString => vec![(json!(""), Arc::new(StringArray::from(vec![""])))],
        E::Null => vec![(Value::Null, Arc::new(StringArray::from(vec![None::<&str>])))],
        E::Boolean => [false, true]
            .into_iter()
            .map(|v| (json!(v), Arc::new(BooleanArray::from(vec![v])) as ArrayRef))
            .collect(),
        E::SignedInteger => [-2_147_483_648_i64, 2_147_483_647]
            .into_iter()
            .map(|v| (json!(v), Arc::new(Int64Array::from(vec![v])) as ArrayRef))
            .collect(),
        E::UnsignedInteger => vec![(json!(u64::MAX), Arc::new(UInt64Array::from(vec![u64::MAX])))],
        E::Floating | E::NumberPrecision => [1.25, -0.0, 1.123_456_789_012_345]
            .into_iter()
            .map(|v| (json!(v), Arc::new(Float64Array::from(vec![v])) as ArrayRef))
            .collect(),
        _ => return None,
    })
}

fn native_base(driver: &str) -> RecordBatch {
    let b = geometry_batch(WkbValue::Point(coordinate(1.0, 2.0)), GeometryType::Point);
    let b = metadata(&b, "crs", "OGC:CRS84", false);
    let b = match driver {
        "csv" => metadata(
            &b,
            "plenora.geometry.native.csv.geometry_encoding",
            "wkt",
            false,
        ),
        "xls" => metadata(
            &metadata(
                &b,
                "plenora.geometry.native.xlsx.geometry_encoding",
                "wkt",
                false,
            ),
            "xlsx.visibility",
            "visible",
            true,
        ),
        _ => b,
    };
    match driver {
        "geojson" => metadata(&b, "plenora.geometry.dimensions", "unknown", false),
        "csv" | "xls" => metadata(&b, "plenora.geometry.axis_order", "unknown", false),
        _ => b,
    }
}
fn with_value(base: &RecordBatch, name: &str, array: ArrayRef) -> RecordBatch {
    let mut fields = base
        .schema()
        .fields()
        .iter()
        .map(|f| f.as_ref().clone())
        .collect::<Vec<_>>();
    fields.push(Field::new(name, array.data_type().clone(), true));
    let mut columns = base.columns().to_vec();
    columns.push(array);
    RecordBatch::try_new(
        Arc::new(Schema::new_with_metadata(
            fields,
            base.schema().metadata().clone(),
        )),
        columns,
    )
    .unwrap()
}

fn read(driver: &str, native: NativeDriver, element: E, root: &Path, row: &mut Value) -> bool {
    if element == E::Dimensions {
        return dimensions_read(driver, native, root, row);
    }
    // KML native attributes are strings. Numeric Arrow types have no native
    // scalar declaration here and must not be labelled as tested rejections.
    let Some(fixtures) = scalar(element) else {
        return rejection_read(driver, element, root, row)
            || document_read(driver, native, element, root, row);
    };
    if driver == "kml" {
        return document_read(driver, native, element, root, row);
    }
    if driver == "xls" && element == E::UnsignedInteger {
        return false;
    }
    let mut failures = Vec::new();
    for (variant, (value, array)) in fixtures.into_iter().enumerate() {
        let name = if element == E::FieldNames {
            "long_field_name_\u{e9}"
        } else {
            "value"
        };
        let array = if driver == "csv"
            && value
                .as_f64()
                .is_some_and(|v| v == 0.0 && v.is_sign_negative())
        {
            Arc::new(StringArray::from(vec!["-0"])) as ArrayRef
        } else if driver == "csv" && element == E::UnsignedInteger {
            // CSV has no unsigned type marker; out-of-i64 decimal tokens remain
            // exact text under the public inference contract.
            Arc::new(StringArray::from(vec![value.to_string()])) as ArrayRef
        } else if driver == "xls" && element == E::SignedInteger {
            // OOXML numeric cells are binary64; they have no integer width tag.
            Arc::new(Float64Array::from(vec![value.as_f64().unwrap()])) as ArrayRef
        } else {
            array
        };
        let before = with_value(&native_base(driver), name, array);
        let token = match &value {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            _ => value.to_string(),
        };
        let bytes = match driver {
            "geojson" => serde_json::to_vec(&json!({"type":"FeatureCollection","features":[{"type":"Feature","properties":{name:value},"geometry":{"type":"Point","coordinates":[1,2]}}]})).unwrap(),
            "csv" => {
                let token = if value.is_f64() { value.as_f64().unwrap().to_string() } else { token };
                let token = if value.is_string() { format!("\"{}\"",token.replace('"',"\"\"")) } else { token };
                format!("geometry,{name}\n\"POINT (1 2)\",{token}\n").into_bytes()
            },
            "xls" => {
                let cell = match &value {
                    Value::Null => String::new(),
                    Value::Bool(v) => format!("<c r=\"B2\" t=\"b\"><v>{}</v></c>",u8::from(*v)),
                    Value::Number(_) => format!("<c r=\"B2\"><v>{token}</v></c>"),
                    _ => format!("<c r=\"B2\" t=\"inlineStr\"><is><t>{}</t></is></c>",xml(&token)),
                };
                group_a_xlsx_document(&format!("<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>geometry</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>{}</t></is></c></row><row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>POINT (1 2)</t></is></c>{cell}</row>",xml(name)))
            }
            _ => unreachable!(),
        };
        let dir = tempfile::tempdir_in(root).unwrap();
        let input = dir.path().join(format!("source.{}", extension(driver)));
        let output = dir.path().join("back.arrow");
        std::fs::write(&input, bytes).unwrap();
        let (ok, report) = run(&input, &output, driver, "ipc");
        if !ok {
            failures.push(json!({"variant":variant,"read_failed":report}));
            continue;
        }
        let after = native_batch(&input, &output, driver);
        let differences = compare(&before, &after);
        let observed = counts(native, D::Read, &differences);
        let declared = report_categories(&report, "read_loss");
        if observed != declared {
            failures.push(json!({"variant":variant,"observed":observed,"declared":declared,"differences":format!("{differences:?}"),"schema":format!("{:?}",after.schema())}));
        }
    }
    finish(row, failures);
    row["witness"] = json!(format!(
        "direct native {driver} scalar document; no IO writer"
    ));
    true
}

struct Document {
    bytes: Vec<u8>,
    faithful: RecordBatch,
    // Semantics present in the native document but absent from its complete
    // returned contract. The ordinary comparator checks that complete contract.
    omitted: Vec<E>,
}

fn kml_base() -> RecordBatch {
    let base = native_base("kml");
    let b = with_value(
        &base,
        "name",
        Arc::new(StringArray::from(vec![None::<&str>])),
    );
    with_value(
        &b,
        "description",
        Arc::new(StringArray::from(vec![None::<&str>])),
    )
}

#[allow(clippy::too_many_lines)]
fn document(driver: &str, element: E) -> Option<Document> {
    if matches!(driver, "csv" | "xls") && matches!(element, E::M | E::EmptyGeometry) {
        return Some(wkt_document(driver, element));
    }
    if element == E::NullGeometry {
        return Some(null_document(driver));
    }
    let intrinsic = matches!(
        element,
        E::ContractVersion
            | E::FieldIdentity
            | E::TypesDeclaration
            | E::GeometryEncoding
            | E::SpatialSemantics
            | E::Precision
            | E::CrsResolution
            | E::AxisOrder
            | E::ExtensionName
            | E::CrsId
            | E::Dimensions
            | E::PrimaryGeometry
    );
    let mut faithful = if driver == "kml" {
        kml_base()
    } else {
        native_base(driver)
    };
    let mut omitted = Vec::new();
    let bytes = match driver {
        "geojson" => {
            let mut feature = json!({"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[1,2]}});
            let mut root = json!({"type":"FeatureCollection"});
            match element {
                _ if intrinsic => {}
                E::FeatureId => { feature["id"] = json!("native-id-47"); omitted.push(element); }
                E::ForeignMembers | E::NativeMetadata => { feature["native-key"] = json!({"payload":[3,7]}); omitted.push(E::ForeignMembers); }
                E::Styles => { feature["style"] = json!({"stroke":"#123456"}); omitted.push(E::ForeignMembers); }
                E::Extensions => { root["extension"] = json!({"version":7}); omitted.push(E::ForeignMembers); }
                E::LayerMetadata => { root["title"] = json!("native collection title"); omitted.push(E::ForeignMembers); }
                E::FeatureMetadata => { feature["title"] = json!("native feature title"); omitted.push(E::ForeignMembers); }
                E::ExternalReference => { feature["link"] = json!("relative/external.json"); omitted.push(E::ForeignMembers); }
                _ => return None,
            }
            root["features"] = json!([feature]);
            serde_json::to_vec(&root).unwrap()
        }
        "kml" => {
            let mut doc = String::new();
            let mut extra = String::new();
            let mut id = "";
            let mut folder = false;
            match element {
                _ if intrinsic => {}
                E::Utf8 | E::EmptyString | E::Null => {
                    let text = match element { E::Utf8 => Some("\u{e8}\u{6f22}\u{5b57}<&"), E::EmptyString => Some(""), _ => None };
                    faithful = with_value(&native_base("kml"),"name",Arc::new(StringArray::from(vec![text])));
                    faithful = with_value(&faithful,"description",Arc::new(StringArray::from(vec![None::<&str>])));
                    if element != E::Null {
                        extra = format!("<name>{}</name>",xml(text.unwrap_or("")));
                    }
                }
                E::FeatureId => { id = " id=\"native-id-47\""; omitted.push(element); }
                E::Styles => { extra = "<styleUrl>#native-style</styleUrl>".into(); omitted.push(element); }
                E::Hierarchy => { folder = true; omitted.push(element); }
                E::FeatureMetadata => { extra = "<visibility>0</visibility>".into(); omitted.push(element); }
                E::LayerMetadata => { doc = "<name>native collection title</name>".into(); omitted.push(element); }
                E::ForeignMembers | E::NativeMetadata | E::Extensions => { extra = "<custom>native payload</custom>".into(); omitted.push(E::ForeignMembers); }
                _ => return None,
            }
            format!("<kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document>{doc}{}<Placemark{id}>{extra}<Point><altitudeMode>absolute</altitudeMode><coordinates>1,2</coordinates></Point></Placemark>{}</Document></kml>", if folder {"<Folder>"} else {""},if folder {"</Folder>"} else {""}).into_bytes()
        }
        "csv" if intrinsic => b"geometry\n\"POINT (1 2)\"\n".to_vec(),
        "xls" if intrinsic => group_a_xlsx_document("<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>geometry</t></is></c></row><row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>POINT (1 2)</t></is></c></row>"),
        "xls" if matches!(element,E::Formula | E::FormulaCache) => {
            faithful = with_value(&faithful,"value",Arc::new(Float64Array::from(vec![42.0])));
            omitted.push(E::Formula);
            group_a_xlsx_document("<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>geometry</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>value</t></is></c></row><row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>POINT (1 2)</t></is></c><c r=\"B2\"><f>6*7</f><v>42</v></c></row>")
        }
        _ => return None,
    };
    Some(Document {
        bytes,
        faithful,
        omitted,
    })
}

fn document_read(
    driver: &str,
    native: NativeDriver,
    element: E,
    root: &Path,
    row: &mut Value,
) -> bool {
    let Some(document) = document(driver, element).or_else(|| second::document(driver, element))
    else {
        return false;
    };
    let dir = tempfile::tempdir_in(root).unwrap();
    let input = dir.path().join(format!("source.{}", extension(driver)));
    let output = dir.path().join("back.arrow");
    std::fs::write(&input, &document.bytes).unwrap();
    let (ok, report) = run(&input, &output, driver, "ipc");
    let mut failures = Vec::new();
    if ok {
        let after = native_batch(&input, &output, driver);
        let differences = compare(&document.faithful, &after);
        let mut observed = counts(native, D::Read, &differences);
        second::normalize_native_order(&differences, &mut observed);
        // A native-only omission is valid only if no unexpected representation
        // appears in the returned schema/columns. Added metadata is inspected too.
        let extra_fields = after.num_columns() != document.faithful.num_columns()
            || (!document.omitted.is_empty() && after.schema() != document.faithful.schema());
        for omitted in &document.omitted {
            if extra_fields {
                failures.push(json!({"unexpected_native_representation":format!("{omitted:?}")}));
            } else {
                *observed
                    .entry(category(
                        native,
                        D::Read,
                        &Difference {
                            element: *omitted,
                            detail: "native semantic object absent from returned contract".into(),
                        },
                    ))
                    .or_default() += 1;
            }
        }
        let declared = report_categories(&report, "read_loss");
        if observed != declared {
            failures.push(json!({"observed":observed,"declared":declared,"differences":format!("{differences:?}"),"schema":format!("{:?}",after.schema())}));
        }
        // Retain the independently constructed contract and exact native bytes
        // in the ledger, making native-object omission claims reviewable.
        row["native_document"] = json!(String::from_utf8_lossy(&document.bytes)
            .chars()
            .take(4096)
            .collect::<String>());
        row["native_omissions"] = json!(document
            .omitted
            .iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>());
    } else {
        failures.push(json!({"read_failed":report}));
    }
    finish(row, failures);
    row["witness"] = json!(format!("direct native {driver} document; no IO writer"));
    true
}

fn rejection_read(driver: &str, element: E, root: &Path, row: &mut Value) -> bool {
    let bytes = match (driver,element) {
        ("geojson",E::M) => r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[1,2,3,4]}}]}"#,
        ("geojson",E::EmptyPoint) => r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[]}}]}"#,
        ("kml",E::M) => "<kml><Document><Placemark><Point><coordinates>1,2,3,4</coordinates></Point></Placemark></Document></kml>",
        ("kml",E::EmptyPoint) => "<kml><Document><Placemark><Point><coordinates></coordinates></Point></Placemark></Document></kml>",
        _ => return false,
    };
    let dir = tempfile::tempdir_in(root).unwrap();
    let input = dir.path().join(format!("source.{}", extension(driver)));
    let output = dir.path().join("back.arrow");
    std::fs::write(&input, bytes).unwrap();
    let (ok, report) = if driver == "kml" {
        // The malformed object is Placemark index 2 (root 0, Document 1).
        // Reading the aggregate root intentionally includes omitted objects;
        // this rejection witness explicitly selects the object under test.
        let output = Command::new(env!("CARGO_BIN_EXE_plenora-io"))
            .arg("convert")
            .arg(&input)
            .arg(&output)
            .args(["--from", "kml", "--to", "ipc", "--layer", "2"])
            .output()
            .unwrap();
        (
            output.status.success(),
            serde_json::from_slice(&output.stdout).unwrap(),
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
    row["native_document"] = json!(bytes);
    row["witness"] =
        json!("direct native invalid coordinate payload; category/code and absent destination");
    true
}

fn null_document(driver: &str) -> Document {
    let base = native_base(driver);
    let point = base
        .column(0)
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap()
        .value(0);
    let geometry: ArrayRef = Arc::new(BinaryArray::from(vec![None, Some(point)]));
    let mut faithful = RecordBatch::try_new(base.schema(), vec![geometry]).unwrap();
    let name = if driver == "kml" { "name" } else { "value" };
    faithful = with_value(
        &faithful,
        name,
        Arc::new(StringArray::from(vec!["null row", "point row"])),
    );
    if driver == "kml" {
        faithful = with_value(
            &faithful,
            "description",
            Arc::new(StringArray::from(vec![None::<&str>, None])),
        );
    }
    let bytes = match driver {
        "geojson" => serde_json::to_vec(&json!({"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"value":"null row"},"geometry":null},
            {"type":"Feature","properties":{"value":"point row"},"geometry":{"type":"Point","coordinates":[1,2]}}
        ]})).unwrap(),
        "csv" => b"geometry,value\n,null row\n\"POINT (1 2)\",point row\n".to_vec(),
        "kml" => b"<kml><Document><Placemark><name>null row</name></Placemark><Placemark><name>point row</name><Point><coordinates>1,2</coordinates></Point></Placemark></Document></kml>".to_vec(),
        "xls" => group_a_xlsx_document("<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>geometry</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>value</t></is></c></row><row r=\"2\"><c r=\"B2\" t=\"inlineStr\"><is><t>null row</t></is></c></row><row r=\"3\"><c r=\"A3\" t=\"inlineStr\"><is><t>POINT (1 2)</t></is></c><c r=\"B3\" t=\"inlineStr\"><is><t>point row</t></is></c></row>"),
        _ => unreachable!(),
    };
    Document {
        bytes,
        faithful,
        omitted: Vec::new(),
    }
}

fn ipc_read(element: E, root: &Path, row: &mut Value) -> bool {
    let Some(fixtures) = factory(element).or_else(|| generate(element).ok()) else {
        return false;
    };
    let mut failures = Vec::new();
    for (variant, before) in fixtures.iter().enumerate() {
        let dir = tempfile::tempdir_in(root).unwrap();
        let input = dir.path().join("native.arrow");
        let output = dir.path().join("result.arrow");
        // Arrow's FileWriter authors the native source; no IO writer precedes
        // this Read witness, including the schema/nested/metadata variants.
        save(&input, before);
        let (ok, report) = run(&input, &output, "ipc", "ipc");
        if !ok {
            failures.push(json!({"variant":variant,"read_failed":report}));
            continue;
        }
        let after = super::super::load(&output);
        let differences = compare(before, &after);
        let observed = counts(NativeDriver::Ipc, D::Read, &differences);
        let declared = report_categories(&report, "read_loss");
        if observed != declared {
            failures.push(json!({"variant":variant,"observed":observed,"declared":declared,"differences":format!("{differences:?}")}));
        }
    }
    finish(row, failures);
    row["witness"] =
        json!("native Arrow FileWriter file; read directly without preceding IO write");
    true
}

pub(super) fn write_audit(root: &Path, rows: &[Value]) {
    let mut counts = BTreeMap::<String, u64>::new();
    let mut per_driver = BTreeMap::<String, BTreeMap<String, u64>>::new();
    let remaining = rows
        .iter()
        .filter(|r| r["status"] != "passed")
        .collect::<Vec<_>>();
    for row in rows {
        let driver = row["driver"].as_str().unwrap();
        let status = row["status"].as_str().unwrap();
        *counts.entry(status.to_owned()).or_default() += 1;
        *per_driver
            .entry(driver.to_owned())
            .or_default()
            .entry(status.to_owned())
            .or_default() += 1;
    }
    let audit =
        json!({"total":rows.len(),"counts":counts,"per_driver":per_driver,"remaining":remaining});
    std::fs::write(
        root.join("audit.json"),
        serde_json::to_vec_pretty(&audit).unwrap(),
    )
    .unwrap();
    eprintln!("Group A operational evidence: {}", root.display());
    for row in remaining {
        eprintln!(
            "{}/{}/{}: {} ({})",
            row["driver"],
            row["direction"],
            row["element"],
            row["status"],
            row.get("reason").unwrap_or(&Value::Null)
        );
    }
}

fn wkt_document(driver: &str, element: E) -> Document {
    let source = generate(element).unwrap().remove(0);
    let mut faithful = metadata(&source, "plenora.geometry.axis_order", "unknown", false);
    faithful = metadata(&faithful, "crs", "OGC:CRS84", false);
    faithful = metadata(
        &faithful,
        if driver == "csv" {
            "plenora.geometry.native.csv.geometry_encoding"
        } else {
            "plenora.geometry.native.xlsx.geometry_encoding"
        },
        "wkt",
        false,
    );
    if driver == "xls" {
        faithful = metadata(&faithful, "xlsx.visibility", "visible", true);
    }
    let text = if element == E::M {
        "POINT M (1 2 3)"
    } else {
        "MULTIPOLYGON EMPTY"
    };
    let bytes = if driver == "csv" {
        format!("geometry\n\"{text}\"\n").into_bytes()
    } else {
        group_a_xlsx_document(&format!("<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>geometry</t></is></c></row><row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>{text}</t></is></c></row>"))
    };
    Document {
        bytes,
        faithful,
        omitted: Vec::new(),
    }
}

fn dimensions_read(driver: &str, native: NativeDriver, root: &Path, row: &mut Value) -> bool {
    let mut z = json!({});
    super::native::exercise(driver, native, E::Z, root, &mut z);
    let mut m = json!({});
    let exercised = rejection_read(driver, E::M, root, &mut m)
        || document_read(driver, native, E::M, root, &mut m);
    assert!(exercised);
    let mut failures = Vec::new();
    if z["status"] != "passed" {
        failures.push(json!({"z":z}));
    }
    if m["status"] != "passed" {
        failures.push(json!({"m":m}));
    }
    finish(row, failures);
    row["witness"] = json!("direct native Z and M documents, including typed M rejections");
    true
}
