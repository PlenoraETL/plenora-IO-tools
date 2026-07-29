//! Codec WKB condiviso.
//!
//! L'AST lossless è l'unica implementazione binaria: conserva Z/M/SRID e
//! applica limiti e validazione strutturale. Le funzioni `geo-types` restano
//! adattatori XY compatibili con l'API v1, senza un secondo parser/encoder.

use geo_types::Geometry;

use crate::error::Result;
use crate::limits::WkbLimits;

pub use crate::wkb_lossless::{
    decode_wkb, encode_wkb, encode_wkb_into, inspect_wkb, WkbCoordinate, WkbFlavor, WkbGeometry,
    WkbInspection, WkbValue,
};

/// Serializza una geometria `geo-types` in WKB XY little-endian, riusando il
/// buffer fornito (che viene svuotato prima della scrittura).
pub fn to_wkb_into(geometry: &Geometry<f64>, output: &mut Vec<u8>) -> Result<()> {
    let geometry = WkbGeometry::from_geo_xy(geometry)?;
    encode_wkb_into(&geometry, WkbFlavor::Iso, output)
}

/// Serializza una geometria `geo-types` in WKB XY little-endian.
pub fn to_wkb(geometry: &Geometry<f64>) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    to_wkb_into(geometry, &mut output)?;
    Ok(output)
}

/// Decodifica WKB ISO/EWKB e proietta intenzionalmente il risultato su XY.
///
/// Per conservare Z, M e SRID usare [`decode_wkb`]. Anche l'adattatore v1 usa
/// il parser autoritativo e quindi rifiuta byte residui e strutture incoerenti.
pub fn from_wkb(bytes: &[u8], limits: &WkbLimits) -> Result<Geometry<f64>> {
    decode_wkb(bytes, limits)?.to_geo_xy()
}

#[cfg(test)]
mod tests {
    use geo_types::{
        Coord, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon, Point,
        Polygon,
    };

    use crate::contract::{CoordinateDimensions, GeometryType};

    use super::*;

    fn sample_geometries() -> Vec<Geometry<f64>> {
        let polygon = Polygon::new(
            LineString(vec![
                Coord { x: 0.0, y: 0.0 },
                Coord { x: 4.0, y: 0.0 },
                Coord { x: 4.0, y: 4.0 },
                Coord { x: 0.0, y: 0.0 },
            ]),
            vec![],
        );
        vec![
            Geometry::Point(Point::new(1.0, 2.0)),
            Geometry::LineString(LineString(vec![
                Coord { x: 0.0, y: 0.0 },
                Coord { x: 1.0, y: 1.0 },
            ])),
            Geometry::Polygon(polygon.clone()),
            Geometry::MultiPoint(MultiPoint(vec![Point::new(0.0, 0.0)])),
            Geometry::MultiLineString(MultiLineString(vec![LineString(vec![
                Coord { x: 0.0, y: 0.0 },
                Coord { x: 1.0, y: 1.0 },
            ])])),
            Geometry::MultiPolygon(MultiPolygon(vec![polygon])),
            Geometry::GeometryCollection(GeometryCollection(vec![Geometry::Point(Point::new(
                7.0, 8.0,
            ))])),
        ]
    }

    fn xy(x: f64, y: f64) -> WkbCoordinate {
        WkbCoordinate {
            x,
            y,
            z: None,
            m: None,
        }
    }

    fn extended_geometries() -> Vec<WkbGeometry> {
        let child = |value| WkbGeometry {
            value,
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        let line = || child(WkbValue::LineString(vec![xy(0.0, 0.0), xy(1.0, 1.0)]));
        let circular = || {
            child(WkbValue::CircularString(vec![
                xy(0.0, 0.0),
                xy(1.0, 1.0),
                xy(2.0, 0.0),
            ]))
        };
        let polygon = || {
            child(WkbValue::Polygon(vec![vec![
                xy(0.0, 0.0),
                xy(2.0, 0.0),
                xy(0.0, 0.0),
            ]]))
        };
        let triangle = || {
            child(WkbValue::Triangle(vec![vec![
                xy(0.0, 0.0),
                xy(2.0, 0.0),
                xy(0.0, 0.0),
            ]]))
        };
        let compound = || child(WkbValue::CompoundCurve(vec![line(), circular()]));
        let curve_polygon = || child(WkbValue::CurvePolygon(vec![circular()]));

        vec![
            circular(),
            compound(),
            curve_polygon(),
            child(WkbValue::MultiCurve(vec![line(), compound()])),
            child(WkbValue::MultiSurface(vec![polygon(), curve_polygon()])),
            child(WkbValue::PolyhedralSurface(vec![polygon()])),
            child(WkbValue::Tin(vec![triangle()])),
            triangle(),
        ]
    }

    #[test]
    fn geo_adapter_roundtrips_every_standard_type() {
        for geometry in sample_geometries() {
            let bytes = to_wkb(&geometry).unwrap();
            assert_eq!(from_wkb(&bytes, &WkbLimits::default()).unwrap(), geometry);
        }
    }

    #[test]
    fn geo_adapter_is_byte_identical_to_authoritative_encoder() {
        for geometry in sample_geometries() {
            let ast = WkbGeometry::from_geo_xy(&geometry).unwrap();
            assert_eq!(
                to_wkb(&geometry).unwrap(),
                encode_wkb(&ast, WkbFlavor::Iso).unwrap()
            );
        }
    }

    #[test]
    fn allocation_free_inspection_matches_the_authoritative_decoder() {
        for geometry in sample_geometries() {
            let bytes = to_wkb(&geometry).unwrap();
            let decoded = decode_wkb(&bytes, &WkbLimits::default()).unwrap();
            let inspected = inspect_wkb(&bytes, &WkbLimits::default()).unwrap();
            assert_eq!(inspected.geometry_type, decoded.geometry_type());
            assert_eq!(inspected.dimensions, decoded.dimensions);
            assert_eq!(inspected.srid, decoded.srid);
            assert!(inspected.nested_dimensions_coherent);
            assert!(!inspected.contains_srid);
        }

        let ewkb = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: Some(3.0),
                m: None,
            }),
            dimensions: CoordinateDimensions::Xyz,
            srid: Some(4326),
        };
        let bytes = encode_wkb(&ewkb, WkbFlavor::Ewkb).unwrap();
        let inspected = inspect_wkb(&bytes, &WkbLimits::default()).unwrap();
        assert_eq!(inspected.geometry_type, GeometryType::Point);
        assert_eq!(inspected.srid, Some(4326));
        assert!(inspected.contains_srid);
    }

    #[test]
    fn lossless_codec_roundtrips_every_concrete_canonical_type() {
        let mut observed = sample_geometries()
            .iter()
            .map(WkbGeometry::from_geo_xy)
            .collect::<Result<Vec<_>>>()
            .unwrap();
        observed.extend(extended_geometries());

        let expected = [
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
        ];
        assert_eq!(observed.len(), expected.len());
        for (geometry, expected_type) in observed.iter().zip(expected) {
            assert_eq!(geometry.geometry_type(), expected_type);
            let bytes = encode_wkb(geometry, WkbFlavor::Iso).unwrap();
            assert_eq!(
                decode_wkb(&bytes, &WkbLimits::default()).unwrap(),
                *geometry
            );
            let inspection = inspect_wkb(&bytes, &WkbLimits::default()).unwrap();
            assert_eq!(inspection.geometry_type, expected_type);
            assert!(inspection.nested_dimensions_coherent);
        }
    }

    #[test]
    fn extended_types_preserve_xyzm_and_srid_in_ewkb() {
        let coordinate = |x, y, z, m| WkbCoordinate {
            x,
            y,
            z: Some(z),
            m: Some(m),
        };
        let geometry = WkbGeometry {
            value: WkbValue::CircularString(vec![
                coordinate(0.0, 0.0, 10.0, 1.0),
                coordinate(1.0, 1.0, 11.0, 2.0),
                coordinate(2.0, 0.0, 12.0, 3.0),
            ]),
            dimensions: CoordinateDimensions::Xyzm,
            srid: Some(4979),
        };
        let bytes = encode_wkb(&geometry, WkbFlavor::Ewkb).unwrap();
        assert_eq!(decode_wkb(&bytes, &WkbLimits::default()).unwrap(), geometry);
        let inspection = inspect_wkb(&bytes, &WkbLimits::default()).unwrap();
        assert_eq!(inspection.geometry_type, GeometryType::CircularString);
        assert_eq!(inspection.dimensions, CoordinateDimensions::Xyzm);
        assert_eq!(inspection.srid, Some(4979));
        assert!(inspection.contains_srid);
    }

    #[test]
    fn extended_aggregates_reject_noncanonical_children() {
        let invalid = WkbGeometry {
            value: WkbValue::Tin(vec![WkbGeometry {
                value: WkbValue::Polygon(vec![]),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            }]),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        assert!(encode_wkb(&invalid, WkbFlavor::Iso).is_err());
    }

    #[test]
    fn geo_adapter_rejects_extended_types_without_linearizing_them() {
        for geometry in extended_geometries() {
            let bytes = encode_wkb(&geometry, WkbFlavor::Iso).unwrap();
            assert!(from_wkb(&bytes, &WkbLimits::default()).is_err());
        }
    }

    #[test]
    fn hostile_input_and_limits_fail_closed() {
        assert!(from_wkb(&[], &WkbLimits::default()).is_err());
        let mut evil = vec![1_u8, 2, 0, 0, 0];
        evil.extend_from_slice(&1_000_000_000_u32.to_le_bytes());
        assert!(from_wkb(&evil, &WkbLimits::default()).is_err());

        let tight = WkbLimits {
            max_components: 1,
            ..WkbLimits::default()
        };
        let line = to_wkb(&Geometry::LineString(LineString(vec![
            Coord { x: 0.0, y: 0.0 },
            Coord { x: 1.0, y: 1.0 },
            Coord { x: 2.0, y: 2.0 },
        ])))
        .unwrap();
        assert!(from_wkb(&line, &tight).is_err());
    }

    #[test]
    fn xy_adapter_uses_lossless_ewkb_parser() {
        let mut ewkb = vec![1];
        ewkb.extend_from_slice(&0xA000_0001_u32.to_le_bytes());
        ewkb.extend_from_slice(&4326_u32.to_le_bytes());
        ewkb.extend_from_slice(&1.0_f64.to_le_bytes());
        ewkb.extend_from_slice(&2.0_f64.to_le_bytes());
        ewkb.extend_from_slice(&3.0_f64.to_le_bytes());

        let decoded = decode_wkb(&ewkb, &WkbLimits::default()).unwrap();
        assert_eq!(decoded.dimensions, CoordinateDimensions::Xyz);
        assert_eq!(decoded.srid, Some(4326));
        assert_eq!(
            decoded.value,
            WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: Some(3.0),
                m: None,
            })
        );
        assert_eq!(encode_wkb(&decoded, WkbFlavor::Ewkb).unwrap(), ewkb);
        assert_eq!(
            from_wkb(&ewkb, &WkbLimits::default()).unwrap(),
            Geometry::Point(Point::new(1.0, 2.0))
        );
    }

    #[test]
    fn iso_xym_and_xyzm_roundtrip_without_loss() {
        for (dimensions, coordinate) in [
            (
                CoordinateDimensions::Xym,
                WkbCoordinate {
                    x: 10.0,
                    y: 20.0,
                    z: None,
                    m: Some(7.0),
                },
            ),
            (
                CoordinateDimensions::Xyzm,
                WkbCoordinate {
                    x: 10.0,
                    y: 20.0,
                    z: Some(30.0),
                    m: Some(7.0),
                },
            ),
        ] {
            let geometry = WkbGeometry {
                value: WkbValue::Point(coordinate),
                dimensions,
                srid: None,
            };
            let encoded = encode_wkb(&geometry, WkbFlavor::Iso).unwrap();
            assert_eq!(
                decode_wkb(&encoded, &WkbLimits::default()).unwrap(),
                geometry
            );
        }
    }

    #[test]
    fn all_decoders_reject_trailing_bytes() {
        let mut bytes = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
        bytes.push(0);
        assert!(decode_wkb(&bytes, &WkbLimits::default()).is_err());
        assert!(inspect_wkb(&bytes, &WkbLimits::default()).is_err());
        assert!(from_wkb(&bytes, &WkbLimits::default()).is_err());
    }

    #[test]
    fn encoder_rejects_incoherent_ordinates() {
        let invalid = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        };
        assert!(encode_wkb(&invalid, WkbFlavor::Iso).is_err());
    }
}
