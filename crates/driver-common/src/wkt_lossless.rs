//! Conversione condivisa fra WKT dimensionale e l'AST WKB lossless.

use plenora_io_model::contract::{CoordinateDimensions, GeometryType};
use plenora_io_model::wkb::{WkbCoordinate, WkbGeometry, WkbValue};
use plenora_io_model::{PlenoraIoError, Result};
use wkt::types::{
    Coord, Dimension, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon,
};
use wkt::Wkt;

fn error(message: impl Into<String>) -> PlenoraIoError {
    PlenoraIoError::Wkb(format!("WKT: {}", message.into()))
}

fn contract_dimensions(dimension: Dimension) -> CoordinateDimensions {
    match dimension {
        Dimension::XY => CoordinateDimensions::Xy,
        Dimension::XYZ => CoordinateDimensions::Xyz,
        Dimension::XYM => CoordinateDimensions::Xym,
        Dimension::XYZM => CoordinateDimensions::Xyzm,
    }
}

fn dimension(dimensions: CoordinateDimensions) -> Result<Dimension> {
    match dimensions {
        CoordinateDimensions::Xy => Ok(Dimension::XY),
        CoordinateDimensions::Xyz => Ok(Dimension::XYZ),
        CoordinateDimensions::Xym => Ok(Dimension::XYM),
        CoordinateDimensions::Xyzm => Ok(Dimension::XYZM),
        CoordinateDimensions::Unknown => Err(error("dimensionalità ignota non serializzabile")),
    }
}

fn validate_finite_coordinate(x: f64, y: f64, z: Option<f64>, m: Option<f64>) -> Result<()> {
    if !x.is_finite()
        || !y.is_finite()
        || z.is_some_and(|value| !value.is_finite())
        || m.is_some_and(|value| !value.is_finite())
    {
        return Err(error("coordinata non finita"));
    }
    Ok(())
}

fn coordinate_from_wkt(
    coordinate: &Coord<f64>,
    expected: CoordinateDimensions,
) -> Result<WkbCoordinate> {
    let actual = contract_dimensions(coordinate.dimension());
    if actual != expected {
        return Err(error(format!(
            "coordinata {actual:?} in geometria {expected:?}"
        )));
    }
    validate_finite_coordinate(coordinate.x, coordinate.y, coordinate.z, coordinate.m)?;
    Ok(WkbCoordinate {
        x: coordinate.x,
        y: coordinate.y,
        z: coordinate.z,
        m: coordinate.m,
    })
}

fn coordinates_from_wkt(
    coordinates: &[Coord<f64>],
    expected: CoordinateDimensions,
) -> Result<Vec<WkbCoordinate>> {
    coordinates
        .iter()
        .map(|coordinate| coordinate_from_wkt(coordinate, expected))
        .collect()
}

fn geometry_from_wkt(value: &Wkt<f64>) -> Result<WkbGeometry> {
    let (value, dimensions) = match value {
        Wkt::Point(point) => {
            let dimensions = contract_dimensions(point.dimension());
            let coordinate = point
                .coord()
                .ok_or_else(|| error("POINT EMPTY non rappresentabile nel core WKB"))?;
            (
                WkbValue::Point(coordinate_from_wkt(coordinate, dimensions)?),
                dimensions,
            )
        }
        Wkt::LineString(line) => {
            let dimensions = contract_dimensions(line.dimension());
            (
                WkbValue::LineString(coordinates_from_wkt(line.coords(), dimensions)?),
                dimensions,
            )
        }
        Wkt::Polygon(polygon) => {
            let dimensions = contract_dimensions(polygon.dimension());
            let rings = polygon
                .rings()
                .iter()
                .map(|ring| {
                    if contract_dimensions(ring.dimension()) != dimensions {
                        return Err(error("anello Polygon con dimensionalità incoerente"));
                    }
                    coordinates_from_wkt(ring.coords(), dimensions)
                })
                .collect::<Result<Vec<_>>>()?;
            (WkbValue::Polygon(rings), dimensions)
        }
        Wkt::MultiPoint(multipoint) => {
            let dimensions = contract_dimensions(multipoint.dimension());
            let children = multipoint
                .points()
                .iter()
                .map(|point| {
                    if contract_dimensions(point.dimension()) != dimensions {
                        return Err(error("Point annidato con dimensionalità incoerente"));
                    }
                    let coordinate = point
                        .coord()
                        .ok_or_else(|| error("POINT EMPTY annidato non rappresentabile"))?;
                    Ok(WkbGeometry {
                        value: WkbValue::Point(coordinate_from_wkt(coordinate, dimensions)?),
                        dimensions,
                        srid: None,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            (WkbValue::MultiPoint(children), dimensions)
        }
        Wkt::MultiLineString(multiline) => {
            let dimensions = contract_dimensions(multiline.dimension());
            let children = multiline
                .line_strings()
                .iter()
                .map(|line| {
                    if contract_dimensions(line.dimension()) != dimensions {
                        return Err(error("LineString annidata con dimensionalità incoerente"));
                    }
                    Ok(WkbGeometry {
                        value: WkbValue::LineString(coordinates_from_wkt(
                            line.coords(),
                            dimensions,
                        )?),
                        dimensions,
                        srid: None,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            (WkbValue::MultiLineString(children), dimensions)
        }
        Wkt::MultiPolygon(multipolygon) => {
            let dimensions = contract_dimensions(multipolygon.dimension());
            let children = multipolygon
                .polygons()
                .iter()
                .map(|polygon| {
                    if contract_dimensions(polygon.dimension()) != dimensions {
                        return Err(error("Polygon annidato con dimensionalità incoerente"));
                    }
                    let rings = polygon
                        .rings()
                        .iter()
                        .map(|ring| {
                            if contract_dimensions(ring.dimension()) != dimensions {
                                return Err(error(
                                    "anello MultiPolygon con dimensionalità incoerente",
                                ));
                            }
                            coordinates_from_wkt(ring.coords(), dimensions)
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(WkbGeometry {
                        value: WkbValue::Polygon(rings),
                        dimensions,
                        srid: None,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            (WkbValue::MultiPolygon(children), dimensions)
        }
        Wkt::GeometryCollection(collection) => {
            let dimensions = contract_dimensions(collection.dimension());
            let children = collection
                .geometries()
                .iter()
                .map(|child| {
                    let child = geometry_from_wkt(child)?;
                    if child.dimensions != dimensions {
                        return Err(error(
                            "GeometryCollection con dimensionalità annidate differenti",
                        ));
                    }
                    Ok(child)
                })
                .collect::<Result<Vec<_>>>()?;
            (WkbValue::GeometryCollection(children), dimensions)
        }
    };
    Ok(WkbGeometry {
        value,
        dimensions,
        srid: None,
    })
}

/// Analizza WKT 2D/3D/M/ZM senza proiettarlo su `geo-types`.
pub fn parse_wkt(text: &str) -> Result<WkbGeometry> {
    let parsed: Wkt<f64> = text
        .parse()
        .map_err(|message| error(format!("sintassi non valida: {message}")))?;
    geometry_from_wkt(&parsed)
}

fn coordinate_to_wkt(
    coordinate: &WkbCoordinate,
    expected: CoordinateDimensions,
) -> Result<Coord<f64>> {
    let actual = match (coordinate.z.is_some(), coordinate.m.is_some()) {
        (false, false) => CoordinateDimensions::Xy,
        (true, false) => CoordinateDimensions::Xyz,
        (false, true) => CoordinateDimensions::Xym,
        (true, true) => CoordinateDimensions::Xyzm,
    };
    if actual != expected {
        return Err(error(format!(
            "coordinata {actual:?} in geometria {expected:?}"
        )));
    }
    validate_finite_coordinate(coordinate.x, coordinate.y, coordinate.z, coordinate.m)?;
    Ok(Coord {
        x: coordinate.x,
        y: coordinate.y,
        z: coordinate.z,
        m: coordinate.m,
    })
}

fn coordinates_to_wkt(
    coordinates: &[WkbCoordinate],
    expected: CoordinateDimensions,
) -> Result<Vec<Coord<f64>>> {
    coordinates
        .iter()
        .map(|coordinate| coordinate_to_wkt(coordinate, expected))
        .collect()
}

fn checked_child<'a>(
    child: &'a WkbGeometry,
    parent: &WkbGeometry,
    expected_type: GeometryType,
) -> Result<&'a WkbValue> {
    if child.srid.is_some()
        || child.dimensions != parent.dimensions
        || child.geometry_type() != expected_type
    {
        return Err(error("geometria WKB annidata incoerente"));
    }
    Ok(&child.value)
}

fn geometry_to_wkt(geometry: &WkbGeometry) -> Result<Wkt<f64>> {
    if geometry.srid.is_some() {
        return Err(error(
            "SRID embedded non rappresentabile in WKT; usare il CRS del contratto",
        ));
    }
    let dim = dimension(geometry.dimensions)?;
    Ok(match &geometry.value {
        WkbValue::Point(coordinate) => Wkt::Point(Point::new(
            Some(coordinate_to_wkt(coordinate, geometry.dimensions)?),
            dim,
        )),
        WkbValue::LineString(coordinates) => Wkt::LineString(LineString::new(
            coordinates_to_wkt(coordinates, geometry.dimensions)?,
            dim,
        )),
        WkbValue::Polygon(rings) => Wkt::Polygon(Polygon::new(
            rings
                .iter()
                .map(|ring| {
                    Ok(LineString::new(
                        coordinates_to_wkt(ring, geometry.dimensions)?,
                        dim,
                    ))
                })
                .collect::<Result<Vec<_>>>()?,
            dim,
        )),
        WkbValue::MultiPoint(children) => Wkt::MultiPoint(MultiPoint::new(
            children
                .iter()
                .map(
                    |child| match checked_child(child, geometry, GeometryType::Point)? {
                        WkbValue::Point(coordinate) => Ok(Point::new(
                            Some(coordinate_to_wkt(coordinate, geometry.dimensions)?),
                            dim,
                        )),
                        _ => Err(error("MultiPoint con membro non-Point")),
                    },
                )
                .collect::<Result<Vec<_>>>()?,
            dim,
        )),
        WkbValue::MultiLineString(children) => Wkt::MultiLineString(MultiLineString::new(
            children
                .iter()
                .map(
                    |child| match checked_child(child, geometry, GeometryType::LineString)? {
                        WkbValue::LineString(coordinates) => Ok(LineString::new(
                            coordinates_to_wkt(coordinates, geometry.dimensions)?,
                            dim,
                        )),
                        _ => Err(error("MultiLineString con membro non-LineString")),
                    },
                )
                .collect::<Result<Vec<_>>>()?,
            dim,
        )),
        WkbValue::MultiPolygon(children) => Wkt::MultiPolygon(MultiPolygon::new(
            children
                .iter()
                .map(
                    |child| match checked_child(child, geometry, GeometryType::Polygon)? {
                        WkbValue::Polygon(rings) => Ok(Polygon::new(
                            rings
                                .iter()
                                .map(|ring| {
                                    Ok(LineString::new(
                                        coordinates_to_wkt(ring, geometry.dimensions)?,
                                        dim,
                                    ))
                                })
                                .collect::<Result<Vec<_>>>()?,
                            dim,
                        )),
                        _ => Err(error("MultiPolygon con membro non-Polygon")),
                    },
                )
                .collect::<Result<Vec<_>>>()?,
            dim,
        )),
        WkbValue::GeometryCollection(children) => {
            let children = children
                .iter()
                .map(|child| {
                    if child.srid.is_some() || child.dimensions != geometry.dimensions {
                        return Err(error(
                            "GeometryCollection WKB con membri dimensionalmente incoerenti",
                        ));
                    }
                    geometry_to_wkt(child)
                })
                .collect::<Result<Vec<_>>>()?;
            Wkt::GeometryCollection(GeometryCollection::new(children, dim))
        }
        WkbValue::CircularString(_)
        | WkbValue::CompoundCurve(_)
        | WkbValue::CurvePolygon(_)
        | WkbValue::MultiCurve(_)
        | WkbValue::MultiSurface(_)
        | WkbValue::PolyhedralSurface(_)
        | WkbValue::Tin(_)
        | WkbValue::Triangle(_) => {
            return Err(error(
                "tipo WKB esteso non rappresentabile dal profilo WKT corrente",
            ))
        }
    })
}

/// Serializza l'AST WKB in WKT dimensionale usando una rappresentazione
/// numerica `f64` round-trip.
pub fn format_wkt(geometry: &WkbGeometry) -> Result<String> {
    let mut output = String::new();
    format_wkt_into(geometry, &mut output)?;
    Ok(output)
}

/// Appende WKT dimensionale a un buffer riusabile.
///
/// La conversione viene validata prima di toccare `output`: in caso di errore
/// il contenuto precedente resta invariato.
pub fn format_wkt_into(geometry: &WkbGeometry, output: &mut String) -> Result<()> {
    use std::fmt::Write as _;

    let wkt = geometry_to_wkt(geometry)?;
    write!(output, "{wkt}")
        .map_err(|format_error| error(format!("serializzazione fallita: {format_error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_all_dimensions_and_nested_geometry() {
        for text in [
            "POINT(1 2)",
            "LINESTRING Z(0 1 2,3 4 5)",
            "MULTIPOINT M((1 2 3),(4 5 6))",
            "MULTIPOLYGON ZM(((0 0 1 10,0 2 2 11,2 0 3 12,0 0 1 10)))",
            "GEOMETRYCOLLECTION Z(POINT Z(1 2 3),LINESTRING Z(0 0 0,1 1 1))",
        ] {
            let geometry = parse_wkt(text).unwrap();
            let encoded = format_wkt(&geometry).unwrap();
            assert_eq!(parse_wkt(&encoded).unwrap(), geometry, "{encoded}");
        }
    }

    #[test]
    fn rejects_empty_point_and_mixed_collection_dimensions() {
        assert!(parse_wkt("POINT EMPTY").is_err());
        assert!(parse_wkt("GEOMETRYCOLLECTION(POINT(1 2),POINT Z(1 2 3))").is_err());
    }

    #[test]
    fn rejects_non_finite_coordinates_from_text_and_wkb() {
        assert!(parse_wkt("POINT (2e308 -1e-308)").is_err());
        assert!(parse_wkt("POINT ZM (1 2 NaN 4)").is_err());

        let geometry = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: f64::INFINITY,
                y: 2.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        assert!(format_wkt(&geometry).is_err());
    }

    #[test]
    fn reusable_formatter_appends_and_preserves_buffer_on_error() {
        let geometry = parse_wkt("LINESTRING Z(0 1 2,3 4 5)").unwrap();
        let mut output = "prefix:".to_owned();
        format_wkt_into(&geometry, &mut output).unwrap();
        assert_eq!(
            parse_wkt(output.strip_prefix("prefix:").unwrap()).unwrap(),
            geometry
        );

        let invalid = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: f64::INFINITY,
                y: 2.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        let before = output.clone();
        assert!(format_wkt_into(&invalid, &mut output).is_err());
        assert_eq!(output, before);
    }
}
