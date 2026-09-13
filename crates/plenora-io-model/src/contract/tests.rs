//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::{CoordinateDimensions, GeometryType};

#[test]
fn geometry_type_wire_names_match_ratified_r3_1() {
    let values = [
        GeometryType::Point,
        GeometryType::LineString,
        GeometryType::Polygon,
        GeometryType::MultiPoint,
        GeometryType::MultiLineString,
        GeometryType::MultiPolygon,
        GeometryType::GeometryCollection,
        GeometryType::CircularString,
        GeometryType::CompoundCurve,
        GeometryType::CurvePolygon,
        GeometryType::MultiCurve,
        GeometryType::MultiSurface,
        GeometryType::PolyhedralSurface,
        GeometryType::Tin,
        GeometryType::Triangle,
        GeometryType::Unknown,
    ];

    assert_eq!(
        serde_json::to_string(&values).unwrap(),
        r#"["point","linestring","polygon","multipoint","multilinestring","multipolygon","geometrycollection","circularstring","compoundcurve","curvepolygon","multicurve","multisurface","polyhedralsurface","tin","triangle","unknown"]"#
    );
    for value in values {
        assert_eq!(
            GeometryType::from_canonical_name(value.canonical_name()),
            Some(value)
        );
    }
}

#[test]
fn coordinate_dimension_wire_names_are_explicitly_lowercase() {
    let values = [
        CoordinateDimensions::Xy,
        CoordinateDimensions::Xyz,
        CoordinateDimensions::Xym,
        CoordinateDimensions::Xyzm,
        CoordinateDimensions::Unknown,
    ];

    assert_eq!(
        serde_json::to_string(&values).unwrap(),
        r#"["xy","xyz","xym","xyzm","unknown"]"#
    );
}
