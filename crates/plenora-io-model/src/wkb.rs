//! Codec WKB condiviso.
//!
//! L'AST lossless è l'unica implementazione binaria: conserva Z/M/SRID e
//! applica limiti e validazione strutturale. Le funzioni `geo-types` restano
//! adattatori XY compatibili con l'API v1, senza un secondo parser/encoder.

use geo_types::Geometry;

use crate::error::Result;
use crate::limits::WkbLimits;

pub use crate::wkb_lossless::{
    decode_wkb, encode_wkb, encode_wkb_into, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
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

    use crate::contract::CoordinateDimensions;

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
