//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use plenora_io_model::contract::CoordinateDimensions;
use plenora_io_model::wkb::{WkbCoordinate, WkbGeometry, WkbValue};
use plenora_io_model::{PlenoraIoError, PublicMessage, Result};
use wkt::types::{Coord, Dimension};
use wkt::Wkt;

fn error(message: &'static str) -> PlenoraIoError {
    PlenoraIoError::wkb_redatto(&PublicMessage::CuratedPair("WKT:", message))
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

const fn contract_dimensions(dimension: Dimension) -> CoordinateDimensions {
    match dimension {
        Dimension::XY => CoordinateDimensions::Xy,
        Dimension::XYZ => CoordinateDimensions::Xyz,
        Dimension::XYM => CoordinateDimensions::Xym,
        Dimension::XYZM => CoordinateDimensions::Xyzm,
    }
}

fn coordinate_from_wkt(
    coordinate: &Coord<f64>,
    expected: CoordinateDimensions,
) -> Result<WkbCoordinate> {
    let actual = contract_dimensions(coordinate.dimension());
    if actual != expected {
        // La dimensionalita' attesa e' quella della geometria, che il
        // chiamante ha in mano: nel messaggio resta quella osservata, che e'
        // l'informazione che lui non ha.
        return Err(PlenoraIoError::wkb_redatto(&PublicMessage::CuratedPair(
            "WKT: coordinata con dimensionalità incoerente con la geometria:",
            actual.nome(),
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

// Dispatch esaustivo sui rami del tipo WKT: la lunghezza e' nel numero di
// varianti, non in complessita' logica.
#[allow(clippy::too_many_lines)]
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

/// Il percorso di lettura come era prima del lotto S12.
pub(super) fn analizza_come_prima(testo: &str) -> Result<WkbGeometry> {
    let albero: Wkt<f64> = testo
        .parse()
        .map_err(|_| error("sintassi WKT non valida"))?;
    let geometria = geometry_from_wkt(&albero)?;
    crate::wkt_lossless::verifica_esprimibile(&geometria)?;
    Ok(geometria)
}
