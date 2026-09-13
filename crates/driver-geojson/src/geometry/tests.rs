//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use geo_types::{
    Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon, Polygon,
};

use super::*;

#[test]
fn geo_writer_rejects_empty_geometries_before_emitting_bytes() {
    let cases = vec![
        Geometry::LineString(LineString::new(Vec::new())),
        Geometry::Polygon(Polygon::new(LineString::new(Vec::new()), Vec::new())),
        Geometry::MultiPoint(MultiPoint(Vec::new())),
        Geometry::MultiLineString(MultiLineString(Vec::new())),
        Geometry::MultiPolygon(MultiPolygon(Vec::new())),
        Geometry::GeometryCollection(GeometryCollection(Vec::new())),
        Geometry::GeometryCollection(GeometryCollection(vec![Geometry::Polygon(Polygon::new(
            LineString::new(Vec::new()),
            Vec::new(),
        ))])),
    ];

    for geometry in cases {
        let mut output = Vec::new();
        assert!(write_geo_geojson(&mut output, &geometry).is_err());
        assert!(output.is_empty());
    }
}

#[test]
fn wkb_writer_rejects_empty_collection_before_emitting_bytes() {
    let geometry = WkbGeometry {
        value: WkbValue::GeometryCollection(Vec::new()),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    let mut output = Vec::new();
    assert!(write_wkb_geojson(&mut output, &geometry).is_err());
    assert!(output.is_empty());
}
