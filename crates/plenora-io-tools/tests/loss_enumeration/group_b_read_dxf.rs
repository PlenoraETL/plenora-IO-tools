//! Native DXF entity, attribute and block witnesses.
use super::*;
use std::fmt::Write;

#[allow(clippy::too_many_lines)] // Keep native group codes beside their independent value/property oracle.
pub(super) fn witness(path: &Path, element: E, changed: bool) -> Witness {
    let source = if element == E::NumberPrecision {
        geometry_batch(
            WkbValue::Point(coordinate(1.123_456_789_012_345, 1e-20)),
            GeometryType::Point,
        )
    } else if element == E::Dimensions && changed {
        generate(E::Z).unwrap().remove(0)
    } else {
        base()
    };
    let mut batch = super::dxf::write(path, &source);
    if element == E::Dimensions {
        batch = metadata(
            &batch,
            "plenora.geometry.dimensions",
            if changed { "xyz" } else { "xy" },
            false,
        );
    }
    let mut text = std::fs::read_to_string(path).unwrap();
    let mut properties = Vec::new();
    let (code, value) = match element {
        E::Color | E::Styles => ("62", "3"),
        E::Linetype => ("6", "DASHED"),
        E::Lineweight => ("370", "25"),
        E::Thickness => ("39", "3.5"),
        E::Handle | E::FeatureId => ("5", "A1"),
        E::Xdata | E::ExtendedData | E::FeatureMetadata | E::LayerMetadata | E::Extensions => {
            ("1001", "NATIVE_TEST\n1000\nindependent-xdata")
        }
        E::PaperSpace | E::Hierarchy => ("67", "1"),
        E::SignedInteger => ("1001", "NATIVE_TEST\n1071\n-2147483648"),
        E::Floating => ("1001", "NATIVE_TEST\n1040\n1.125"),
        _ => ("", ""),
    };
    if changed && !code.is_empty() {
        text = text.replacen("0\nPOINT\n", &format!("0\nPOINT\n{code}\n{value}\n"), 1);
        properties.push((
            if matches!(element, E::SignedInteger | E::Floating) {
                E::Xdata
            } else {
                element
            },
            value.to_owned(),
        ));
    }
    if matches!(
        element,
        E::Utf8 | E::EmptyString | E::TextStyle | E::Null | E::GeometryType | E::MissingValue
    ) {
        let value = if element == E::Utf8 {
            if changed {
                "  caf\u{e9}  "
            } else {
                "caf\u{e9}"
            }
        } else {
            ""
        };
        if !matches!(element, E::Null | E::MissingValue) {
            text = text.replacen("0\nPOINT\n", "0\nTEXT\n", 1);
            text = text.replacen(
                "8\n0\n",
                &format!("8\n0\n1\n{value}\n40\n2.5\n7\nNATIVE_STYLE\n"),
                1,
            );
            let mut arrays = batch.columns().to_vec();
            arrays[2] = Arc::new(StringArray::from(vec!["TEXT"]));
            arrays[3] = Arc::new(StringArray::from(vec![value]));
            batch = RecordBatch::try_new(batch.schema(), arrays).unwrap();
            properties.push((E::TextStyle, "NATIVE_STYLE".to_owned()));
            properties.push((E::GeometryType, "TEXT".to_owned()));
        }
    }
    if element == E::Width {
        let width = if changed { "3.5" } else { "0" };
        let entity =
            format!("0\nLWPOLYLINE\n8\n0\n90\n2\n70\n0\n43\n{width}\n10\n1\n20\n2\n10\n3\n20\n4\n");
        let start = text.find("0\nPOINT\n").unwrap();
        let end = start + text[start..].find("0\nENDSEC\n").unwrap();
        text.replace_range(start..end, &entity);
        let line = geometry_batch(
            WkbValue::LineString(vec![coordinate(1.0, 2.0), coordinate(3.0, 4.0)]),
            GeometryType::LineString,
        );
        let mut arrays = batch.columns().to_vec();
        arrays[0] = line.column(0).clone();
        arrays[2] = Arc::new(StringArray::from(vec!["LWPOLYLINE"]));
        batch = RecordBatch::try_new(batch.schema(), arrays).unwrap();
        if changed {
            properties.push((element, width.to_owned()));
        }
    }
    if matches!(element, E::Boolean | E::RingOrientation) {
        let mut points = vec![
            coordinate(0.0, 0.0),
            coordinate(3.0, 0.0),
            coordinate(3.0, 2.0),
        ];
        if changed {
            points.reverse();
        }
        let closed = element == E::RingOrientation || changed;
        let mut entity = format!("0\nLWPOLYLINE\n8\n0\n90\n3\n70\n{}\n", u8::from(closed));
        for c in &points {
            write!(entity, "10\n{}\n20\n{}\n", c.x, c.y).unwrap();
        }
        let start = text.find("0\nPOINT\n").unwrap();
        let end = start + text[start..].find("0\nENDSEC\n").unwrap();
        text.replace_range(start..end, &entity);
        let (value, kind) = if closed {
            points.push(points[0]);
            (WkbValue::Polygon(vec![points]), GeometryType::Polygon)
        } else {
            (WkbValue::LineString(points), GeometryType::LineString)
        };
        let shape = geometry_batch(value, kind);
        let mut arrays = batch.columns().to_vec();
        arrays[0] = shape.column(0).clone();
        arrays[2] = Arc::new(StringArray::from(vec!["LWPOLYLINE"]));
        batch = RecordBatch::try_new(batch.schema(), arrays).unwrap();
    }
    if matches!(
        element,
        E::FieldNames | E::FieldOrder | E::ExternalReference
    ) {
        let external = element == E::ExternalReference;
        let block = format!("0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n8\n0\n2\nNATIVE_BLOCK\n70\n{}\n10\n0\n20\n0\n30\n0\n3\nNATIVE_BLOCK\n1\nrelative.dwg\n0\nENDBLK\n8\n0\n0\nENDSEC\n", if external { 4 } else { 0 });
        text = text.replacen(
            "0\nSECTION\n2\nENTITIES\n",
            &format!("{block}0\nSECTION\n2\nENTITIES\n"),
            1,
        );
        let mut insert = "0\nINSERT\n8\n0\n2\nNATIVE_BLOCK\n10\n1\n20\n2\n30\n0\n".to_owned();
        if external {
            text = text.replacen(
                "0\nENDSEC\n0\nSECTION\n2\nOBJECTS",
                &format!("{insert}0\nENDSEC\n0\nSECTION\n2\nOBJECTS"),
                1,
            );
            properties.push((element, "relative.dwg".to_owned()));
        } else {
            insert.push_str("66\n1\n");
            let tags = if changed {
                ["zeta", "alpha"]
            } else {
                ["alpha", "zeta"]
            };
            for tag in tags {
                write!(
                    insert,
                    "0\nATTRIB\n8\n0\n10\n1\n20\n2\n40\n1\n1\nvalue-{tag}\n2\n{tag}\n70\n0\n"
                )
                .unwrap();
            }
            insert.push_str("0\nSEQEND\n8\n0\n");
            let start = text.find("0\nPOINT\n").unwrap();
            let end = start + text[start..].find("0\nENDSEC\n").unwrap();
            text.replace_range(start..end, &insert);
            let mut arrays = batch.columns().to_vec();
            arrays[2] = Arc::new(StringArray::from(vec!["INSERT"]));
            arrays[3] = Arc::new(StringArray::from(vec!["NATIVE_BLOCK"]));
            batch = RecordBatch::try_new(batch.schema(), arrays).unwrap();
            properties.push((element, "two-native-attributes".to_owned()));
        }
    }
    if element == E::NativeMetadata {
        batch = metadata(
            &batch,
            "plenora.geometry.native.dxf.geometry_model",
            "wcs",
            false,
        );
        batch = metadata(
            &batch,
            "plenora.geometry.native.dxf.z_inference",
            "xyz_if_any_nonzero_z_else_xy",
            false,
        );
    }
    std::fs::write(path, text).unwrap();
    Witness { batch, properties }
}
