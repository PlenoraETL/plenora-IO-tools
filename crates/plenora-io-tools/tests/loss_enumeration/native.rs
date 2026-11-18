//! Native documents are authored directly, never through an IO writer.
#[path = "xlsx_native.rs"]
mod xlsx_native;
use super::{compare, decode_wkb, generate, metadata, native_batch, report_categories, run};
use super::{json, NativeDriver, Path, Value, WkbLimits, WkbValue, D, E};

fn geojson(value: &WkbValue) -> Option<Value> {
    let coordinate = |c: &plenora_io_model::wkb::WkbCoordinate| {
        let mut values = vec![c.x, c.y];
        values.extend(c.z);
        values
    };
    Some(match value {
        WkbValue::Point(c) if c.x.is_finite() && c.y.is_finite() && c.m.is_none() => {
            json!({"type":"Point", "coordinates":coordinate(c)})
        }
        WkbValue::Polygon(rings) => {
            json!({"type":"Polygon", "coordinates":rings.iter().map(|r| r.iter().map(coordinate).collect::<Vec<_>>()).collect::<Vec<_>>()})
        }
        _ => return None,
    })
}

pub(super) fn exercise(
    driver: &str,
    native: NativeDriver,
    element: E,
    root: &Path,
    row: &mut Value,
) {
    use arrow_array::Array;
    if super::group_b::exercise(driver, native, element, root, row) {
        return;
    }
    if !matches!(driver, "geojson" | "csv" | "xls" | "kml")
        || !matches!(
            element,
            E::GeometryType
                | E::RingOrientation
                | E::SignedZero
                | E::CoordinatePrecision
                | E::Z
                | E::UnknownDimensions
        )
        || (driver != "geojson" && element == E::UnknownDimensions)
    {
        row["status"] = json!("missing-native-witness");
        row["reason"] = json!(format!(
            "native {driver} adapter for {element:?} not implemented"
        ));
        return;
    }
    let mut failures = Vec::new();
    let fixtures = native_fixtures(element);
    assert!(!fixtures.is_empty());
    for (variant, source) in fixtures.iter().enumerate() {
        let column = source
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::BinaryArray>()
            .unwrap();
        let geometry = decode_wkb(column.value(0), &WkbLimits::default()).unwrap();
        let document = json!({"type":"FeatureCollection", "features":[{"type":"Feature", "properties":{}, "geometry":geojson(&geometry.value).unwrap()}]});
        let dir = tempfile::tempdir_in(root).unwrap();
        let input = dir
            .path()
            .join(format!("source.{}", super::extension(driver)));
        let output = dir.path().join("result.arrow");
        let text = wkt(&geometry.value);
        let bytes = match driver {
            "geojson" => serde_json::to_vec(&document).unwrap(),
            "csv" => format!("geometry\n\"{text}\"\n").into_bytes(),
            "xls" => xlsx_native::xlsx_con(&format!("<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>geometry</t></is></c></row><row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>{text}</t></is></c></row>")),
            "kml" => format!("<kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document><Placemark>{}</Placemark></Document></kml>", kml(&geometry.value)).into_bytes(),
            _ => unreachable!(),
        };
        std::fs::write(&input, bytes).unwrap();
        let (ok, report) = run(&input, &output, driver, "ipc");
        if !ok {
            failures.push(json!({"variant":variant,"native_read_failed":report}));
            continue;
        }
        // RFC 7946 coordinates carry ordinates, but no column-wide dimension
        // declaration. Preserve WKB ordinates and compare that declaration as unknown.
        let before = match driver {
            "geojson" => metadata(source, "plenora.geometry.dimensions", "unknown", false),
            "kml" => {
                let fields = vec![
                    source.schema().field(0).clone(),
                    arrow_schema::Field::new("name", arrow_schema::DataType::Utf8, true),
                    arrow_schema::Field::new("description", arrow_schema::DataType::Utf8, true),
                ];
                arrow_array::RecordBatch::try_new(
                    std::sync::Arc::new(arrow_schema::Schema::new_with_metadata(
                        fields,
                        source.schema().metadata().clone(),
                    )),
                    vec![
                        source.column(0).clone(),
                        std::sync::Arc::new(arrow_array::StringArray::from(vec![None::<&str>])),
                        std::sync::Arc::new(arrow_array::StringArray::from(vec![None::<&str>])),
                    ],
                )
                .unwrap()
            }
            _ => metadata(source, "plenora.geometry.axis_order", "unknown", false),
        };
        let after = native_batch(&input, &output, driver);
        let differences = compare(&before, &after);
        let mut observed = std::collections::BTreeMap::<String, u64>::new();
        for difference in &differences {
            *observed
                .entry(super::category(native, D::Read, difference))
                .or_default() += 1;
        }
        let declared = report_categories(&report, "read_loss");
        if observed != declared {
            failures.push(json!({"variant":variant,"observed":observed,"declared":declared,"differences":format!("{differences:?}")}));
        }
    }
    row["status"] = json!(if failures.is_empty() {
        "passed"
    } else {
        "failed"
    });
    row["witness"] = json!(format!("direct native {driver} document; no IO writer"));
    row["failures"] = json!(failures);
}

fn tuple(c: &plenora_io_model::wkb::WkbCoordinate, separator: &str) -> String {
    [Some(c.x), Some(c.y), c.z]
        .into_iter()
        .flatten()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(separator)
}

fn wkt(value: &WkbValue) -> String {
    match value {
        WkbValue::Point(c) => format!(
            "POINT{} ({})",
            if c.z.is_some() { " Z" } else { "" },
            tuple(c, " ")
        ),
        WkbValue::Polygon(rings) => format!(
            "POLYGON ({})",
            rings
                .iter()
                .map(|ring| format!(
                    "({})",
                    ring.iter()
                        .map(|c| tuple(c, " "))
                        .collect::<Vec<_>>()
                        .join(",")
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => unreachable!(),
    }
}

// The Placemark/Document structure follows raccolta-a/kml_gen.py. Unlike that
// exploratory script, every output is paired with independent value assertions.
fn kml(value: &WkbValue) -> String {
    match value {
        WkbValue::Point(c) => format!(
            "<Point><altitudeMode>absolute</altitudeMode><coordinates>{}</coordinates></Point>",
            tuple(c, ",")
        ),
        WkbValue::Polygon(rings) => {
            use std::fmt::Write;
            let mut document = String::from("<Polygon>");
            for (index, ring) in rings.iter().enumerate() {
                let boundary = if index == 0 {
                    "outerBoundaryIs"
                } else {
                    "innerBoundaryIs"
                };
                let coordinates = ring
                    .iter()
                    .map(|c| tuple(c, ","))
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(document, "<{boundary}><LinearRing><coordinates>{coordinates}</coordinates></LinearRing></{boundary}>").unwrap();
            }
            document.push_str("</Polygon>");
            document
        }
        _ => unreachable!(),
    }
}

fn native_fixtures(element: E) -> Vec<arrow_array::RecordBatch> {
    // Native witnesses in this module deliberately cover Point/Polygon.
    // Multipart write fixtures have their own representation-domain checks.
    generate(element)
        .unwrap()
        .into_iter()
        .filter(|source| {
            let column = source
                .column(0)
                .as_any()
                .downcast_ref::<arrow_array::BinaryArray>()
                .unwrap();
            matches!(
                decode_wkb(column.value(0), &WkbLimits::default())
                    .unwrap()
                    .value,
                WkbValue::Point(_) | WkbValue::Polygon(_)
            )
        })
        .collect::<Vec<_>>()
}
