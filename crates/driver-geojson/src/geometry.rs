use std::io::Write;

use plenora_io_model::contract::CoordinateDimensions;
use plenora_io_model::wkb::{
    encode_wkb_into_bounded, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
};
use plenora_io_model::{PlenoraIoError, Result};

fn format_error(reason: impl Into<String>) -> PlenoraIoError {
    PlenoraIoError::format("geojson", reason)
}

fn position(
    ordinates: &[f64],
) -> std::result::Result<(WkbCoordinate, CoordinateDimensions), String> {
    let dimensions = match ordinates.len() {
        2 => CoordinateDimensions::Xy,
        3 => CoordinateDimensions::Xyz,
        n => {
            return Err(format!(
                "posizione GeoJSON con {n} ordinate: attese esattamente 2 o 3"
            ))
        }
    };
    if ordinates.iter().any(|ordinate| !ordinate.is_finite()) {
        return Err("posizione GeoJSON con ordinata non finita".to_owned());
    }
    Ok((
        WkbCoordinate {
            x: ordinates[0],
            y: ordinates[1],
            z: ordinates.get(2).copied(),
            m: None,
        },
        dimensions,
    ))
}

fn positions(
    values: &[Vec<f64>],
) -> std::result::Result<(Vec<WkbCoordinate>, CoordinateDimensions), String> {
    let mut coordinates = Vec::with_capacity(values.len());
    let mut dimensions = None;
    for value in values {
        let (coordinate, current) = position(value)?;
        require_uniform_dimensions(&mut dimensions, current)?;
        coordinates.push(coordinate);
    }
    let dimensions = dimensions.ok_or_else(|| "geometria GeoJSON senza coordinate".to_owned())?;
    Ok((coordinates, dimensions))
}

fn polygon_coordinates(
    values: &[Vec<Vec<f64>>],
) -> std::result::Result<(Vec<Vec<WkbCoordinate>>, CoordinateDimensions), String> {
    let mut rings = Vec::with_capacity(values.len());
    let mut dimensions = None;
    for value in values {
        let (ring, current) = positions(value)?;
        require_uniform_dimensions(&mut dimensions, current)?;
        rings.push(ring);
    }
    let dimensions = dimensions.ok_or_else(|| "Polygon GeoJSON senza anelli".to_owned())?;
    Ok((rings, dimensions))
}

fn require_uniform_dimensions(
    known: &mut Option<CoordinateDimensions>,
    current: CoordinateDimensions,
) -> std::result::Result<(), String> {
    match known {
        Some(value) if *value != current => Err("dimensionalità GeoJSON non uniforme".to_owned()),
        Some(_) => Ok(()),
        None => {
            *known = Some(current);
            Ok(())
        }
    }
}

fn geometry_dimensions(
    geometries: &[WkbGeometry],
    empty_error: &str,
) -> std::result::Result<CoordinateDimensions, String> {
    let dimensions = geometries
        .first()
        .map(|geometry| geometry.dimensions)
        .ok_or_else(|| empty_error.to_owned())?;
    if geometries
        .iter()
        .any(|geometry| geometry.dimensions != dimensions)
    {
        return Err("dimensionalità GeoJSON non uniforme".to_owned());
    }
    Ok(dimensions)
}

fn line_geometry(values: &[Vec<f64>]) -> std::result::Result<WkbGeometry, String> {
    let (coordinates, dimensions) = positions(values)?;
    Ok(WkbGeometry {
        value: WkbValue::LineString(coordinates),
        dimensions,
        srid: None,
    })
}

fn polygon_geometry(values: &[Vec<Vec<f64>>]) -> std::result::Result<WkbGeometry, String> {
    let (rings, dimensions) = polygon_coordinates(values)?;
    Ok(WkbGeometry {
        value: WkbValue::Polygon(rings),
        dimensions,
        srid: None,
    })
}

fn convert(value: &geojson::Value) -> std::result::Result<WkbGeometry, String> {
    use geojson::Value::{
        GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon,
    };

    let (value, dimensions) = match value {
        Point(value) => {
            let (coordinate, dimensions) = position(value)?;
            (WkbValue::Point(coordinate), dimensions)
        }
        LineString(values) => return line_geometry(values),
        Polygon(values) => return polygon_geometry(values),
        MultiPoint(values) => {
            let (coordinates, dimensions) = positions(values)?;
            let geometries = coordinates
                .into_iter()
                .map(|coordinate| WkbGeometry {
                    value: WkbValue::Point(coordinate),
                    dimensions,
                    srid: None,
                })
                .collect();
            (WkbValue::MultiPoint(geometries), dimensions)
        }
        MultiLineString(values) => {
            let geometries = values
                .iter()
                .map(|value| line_geometry(value))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let dimensions = geometry_dimensions(&geometries, "MultiLineString GeoJSON vuota")?;
            (WkbValue::MultiLineString(geometries), dimensions)
        }
        MultiPolygon(values) => {
            let geometries = values
                .iter()
                .map(|value| polygon_geometry(value))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let dimensions = geometry_dimensions(&geometries, "MultiPolygon GeoJSON vuota")?;
            (WkbValue::MultiPolygon(geometries), dimensions)
        }
        GeometryCollection(values) => {
            let geometries = values
                .iter()
                .map(|geometry| convert(&geometry.value))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let dimensions = geometry_dimensions(&geometries, "GeometryCollection GeoJSON vuota")?;
            (WkbValue::GeometryCollection(geometries), dimensions)
        }
    };
    Ok(WkbGeometry {
        value,
        dimensions,
        srid: None,
    })
}

/// Emette WKB ISO XY/XYZ little-endian da una `geojson::Value` (lettura), senza
/// passare da geo_types. Le posizioni devono avere dimensionalità uniforme:
/// una quarta ordinata è rifiutata perché GeoJSON non le assegna una semantica
/// M interoperabile.
#[doc(hidden)] // esposto anche per il fuzzer (plenora-fuzz)
/// Converte una geometria `GeoJSON` in WKB, **senza superare** `max_bytes`.
///
/// Il tetto e' quello configurato: il testo JSON grezzo e' gia' stato
/// controllato dal chiamante, ma la codifica WKB puo' essere piu' grande di
/// quel testo — una `LineString` con poche cifre per coordinata occupa meno
/// caratteri di quanti byte servano ai suoi `f64`.
///
/// Su errore `out` e' lasciato **vuoto**, come in
/// [`encode_wkb_into_bounded`]: la postcondizione e' uniforme, il buffer
/// contiene una codifica completa oppure niente. Vale anche quando a fallire
/// e' la conversione, prima che venga scritto un solo byte.
pub fn wkb_from_gj_value(
    value: &geojson::Value,
    out: &mut Vec<u8>,
    max_bytes: usize,
) -> std::result::Result<(), String> {
    let geometry = convert(value).inspect_err(|_| out.clear())?;
    encode_wkb_into_bounded(&geometry, WkbFlavor::Iso, out, max_bytes)
        .map_err(|error| error.to_string())
}

/// Scrive direttamente il modello WKB lossless come `GeoJSON`, preservando Z.
pub fn write_wkb_geojson<W: Write>(writer: &mut W, geometry: &WkbGeometry) -> Result<()> {
    validate_wkb_geojson_geometry(geometry)?;
    write_wkb_geojson_unchecked(writer, geometry)
}

fn validate_wkb_geojson_geometry(geometry: &WkbGeometry) -> Result<()> {
    if !matches!(
        geometry.dimensions,
        CoordinateDimensions::Xy | CoordinateDimensions::Xyz
    ) {
        return Err(format_error("GeoJSON supporta solo coordinate XY o XYZ"));
    }
    match &geometry.value {
        WkbValue::Point(coordinate) => validate_wkb_coordinate(coordinate),
        WkbValue::LineString(coordinates) => {
            validate_nonempty(coordinates, "LineString WKB vuota")?;
            coordinates.iter().try_for_each(validate_wkb_coordinate)
        }
        WkbValue::Polygon(rings) => {
            validate_nonempty(rings, "Polygon WKB senza anelli")?;
            for ring in rings {
                validate_nonempty(ring, "Polygon WKB con anello vuoto")?;
                ring.iter().try_for_each(validate_wkb_coordinate)?;
            }
            Ok(())
        }
        WkbValue::MultiPoint(points) => {
            validate_nonempty(points, "geometria WKB multipart/collection vuota")?;
            for point in points {
                if !matches!(point.value, WkbValue::Point(_)) {
                    return Err(format_error("MultiPoint WKB con membro non-Point"));
                }
                validate_wkb_child(geometry, point)?;
            }
            Ok(())
        }
        WkbValue::MultiLineString(lines) => {
            validate_nonempty(lines, "geometria WKB multipart/collection vuota")?;
            for line in lines {
                if !matches!(line.value, WkbValue::LineString(_)) {
                    return Err(format_error(
                        "MultiLineString WKB con membro non-LineString",
                    ));
                }
                validate_wkb_child(geometry, line)?;
            }
            Ok(())
        }
        WkbValue::MultiPolygon(polygons) => {
            validate_nonempty(polygons, "geometria WKB multipart/collection vuota")?;
            for polygon in polygons {
                if !matches!(polygon.value, WkbValue::Polygon(_)) {
                    return Err(format_error("MultiPolygon WKB con membro non-Polygon"));
                }
                validate_wkb_child(geometry, polygon)?;
            }
            Ok(())
        }
        WkbValue::GeometryCollection(geometries) => {
            validate_nonempty(geometries, "geometria WKB multipart/collection vuota")?;
            for child in geometries {
                validate_wkb_child(geometry, child)?;
            }
            Ok(())
        }
        WkbValue::CircularString(_)
        | WkbValue::CompoundCurve(_)
        | WkbValue::CurvePolygon(_)
        | WkbValue::MultiCurve(_)
        | WkbValue::MultiSurface(_)
        | WkbValue::PolyhedralSurface(_)
        | WkbValue::Tin(_)
        | WkbValue::Triangle(_) => Err(format_error(
            "tipo WKB esteso non rappresentabile in GeoJSON senza linearizzazione",
        )),
    }
}

fn validate_wkb_child(parent: &WkbGeometry, child: &WkbGeometry) -> Result<()> {
    if child.dimensions != parent.dimensions || child.srid.is_some() {
        return Err(format_error(
            "geometria WKB annidata con dimensioni o SRID incoerenti",
        ));
    }
    validate_wkb_geojson_geometry(child)
}

fn validate_wkb_coordinate(coordinate: &WkbCoordinate) -> Result<()> {
    if !coordinate.x.is_finite()
        || !coordinate.y.is_finite()
        || coordinate.z.is_some_and(|z| !z.is_finite())
    {
        return Err(format_error(
            "coordinata non finita non rappresentabile in GeoJSON",
        ));
    }
    Ok(())
}

fn validate_nonempty<T>(values: &[T], message: &str) -> Result<()> {
    if values.is_empty() {
        return Err(format_error(message));
    }
    Ok(())
}

fn write_wkb_geojson_unchecked<W: Write>(writer: &mut W, geometry: &WkbGeometry) -> Result<()> {
    let dimensions = geometry.dimensions;
    match &geometry.value {
        WkbValue::Point(coordinate) => {
            writer.write_all(b"{\"type\":\"Point\",\"coordinates\":")?;
            write_wkb_position(writer, coordinate, dimensions)?;
            writer.write_all(b"}")?;
        }
        WkbValue::LineString(coordinates) => {
            writer.write_all(b"{\"type\":\"LineString\",\"coordinates\":")?;
            write_wkb_positions(writer, coordinates, dimensions)?;
            writer.write_all(b"}")?;
        }
        WkbValue::Polygon(rings) => {
            writer.write_all(b"{\"type\":\"Polygon\",\"coordinates\":")?;
            write_wkb_polygon(writer, rings, dimensions)?;
            writer.write_all(b"}")?;
        }
        WkbValue::MultiPoint(points) => {
            writer.write_all(b"{\"type\":\"MultiPoint\",\"coordinates\":[")?;
            for (index, point) in points.iter().enumerate() {
                write_separator(writer, index)?;
                let WkbValue::Point(coordinate) = &point.value else {
                    return Err(format_error("MultiPoint WKB con membro non-Point"));
                };
                write_wkb_position(writer, coordinate, dimensions)?;
            }
            writer.write_all(b"]}")?;
        }
        WkbValue::MultiLineString(lines) => {
            writer.write_all(b"{\"type\":\"MultiLineString\",\"coordinates\":[")?;
            for (index, line) in lines.iter().enumerate() {
                write_separator(writer, index)?;
                let WkbValue::LineString(coordinates) = &line.value else {
                    return Err(format_error(
                        "MultiLineString WKB con membro non-LineString",
                    ));
                };
                write_wkb_positions(writer, coordinates, dimensions)?;
            }
            writer.write_all(b"]}")?;
        }
        WkbValue::MultiPolygon(polygons) => {
            writer.write_all(b"{\"type\":\"MultiPolygon\",\"coordinates\":[")?;
            for (index, polygon) in polygons.iter().enumerate() {
                write_separator(writer, index)?;
                let WkbValue::Polygon(rings) = &polygon.value else {
                    return Err(format_error("MultiPolygon WKB con membro non-Polygon"));
                };
                write_wkb_polygon(writer, rings, dimensions)?;
            }
            writer.write_all(b"]}")?;
        }
        WkbValue::GeometryCollection(geometries) => {
            writer.write_all(b"{\"type\":\"GeometryCollection\",\"geometries\":[")?;
            for (index, value) in geometries.iter().enumerate() {
                write_separator(writer, index)?;
                write_wkb_geojson_unchecked(writer, value)?;
            }
            writer.write_all(b"]}")?;
        }
        WkbValue::CircularString(_)
        | WkbValue::CompoundCurve(_)
        | WkbValue::CurvePolygon(_)
        | WkbValue::MultiCurve(_)
        | WkbValue::MultiSurface(_)
        | WkbValue::PolyhedralSurface(_)
        | WkbValue::Tin(_)
        | WkbValue::Triangle(_) => {
            return Err(format_error(
                "tipo WKB esteso non rappresentabile in GeoJSON senza linearizzazione",
            ))
        }
    }
    Ok(())
}

fn write_wkb_position<W: Write>(
    writer: &mut W,
    coordinate: &WkbCoordinate,
    dimensions: CoordinateDimensions,
) -> Result<()> {
    writer.write_all(b"[")?;
    serde_json::to_writer(&mut *writer, &coordinate.x)
        .map_err(|error| format_error(error.to_string()))?;
    writer.write_all(b",")?;
    serde_json::to_writer(&mut *writer, &coordinate.y)
        .map_err(|error| format_error(error.to_string()))?;
    if dimensions == CoordinateDimensions::Xyz {
        writer.write_all(b",")?;
        serde_json::to_writer(
            &mut *writer,
            &coordinate
                .z
                .ok_or_else(|| format_error("coordinata XYZ senza ordinata z"))?,
        )
        .map_err(|error| format_error(error.to_string()))?;
    }
    writer.write_all(b"]")?;
    Ok(())
}

fn write_wkb_positions<W: Write>(
    writer: &mut W,
    coordinates: &[WkbCoordinate],
    dimensions: CoordinateDimensions,
) -> Result<()> {
    writer.write_all(b"[")?;
    for (index, coordinate) in coordinates.iter().enumerate() {
        write_separator(writer, index)?;
        write_wkb_position(writer, coordinate, dimensions)?;
    }
    writer.write_all(b"]")?;
    Ok(())
}

fn write_wkb_polygon<W: Write>(
    writer: &mut W,
    rings: &[Vec<WkbCoordinate>],
    dimensions: CoordinateDimensions,
) -> Result<()> {
    writer.write_all(b"[")?;
    for (index, ring) in rings.iter().enumerate() {
        write_separator(writer, index)?;
        write_wkb_positions(writer, ring, dimensions)?;
    }
    writer.write_all(b"]")?;
    Ok(())
}

/// Scrive una geometria geo_types come oggetto GeoJSON direttamente nel writer.
#[doc(hidden)] // esposto anche per il fuzzer (plenora-fuzz)
pub fn write_geo_geojson<W: Write>(
    writer: &mut W,
    geometry: &geo_types::Geometry<f64>,
) -> Result<()> {
    validate_geo_geometry(geometry)?;
    write_geo_geojson_unchecked(writer, geometry)
}

fn validate_geo_geometry(geometry: &geo_types::Geometry<f64>) -> Result<()> {
    use geo_types::Geometry;

    match geometry {
        Geometry::Point(point) => validate_xy(point.x(), point.y()),
        Geometry::LineString(line) => validate_geo_line(line),
        Geometry::Polygon(polygon) => validate_geo_polygon(polygon),
        Geometry::MultiPoint(points) => {
            validate_nonempty(&points.0, "MultiPoint GeoJSON vuota")?;
            points
                .0
                .iter()
                .try_for_each(|point| validate_xy(point.x(), point.y()))
        }
        Geometry::MultiLineString(lines) => {
            validate_nonempty(&lines.0, "MultiLineString GeoJSON vuota")?;
            lines.0.iter().try_for_each(validate_geo_line)
        }
        Geometry::MultiPolygon(polygons) => {
            validate_nonempty(&polygons.0, "MultiPolygon GeoJSON vuota")?;
            polygons.0.iter().try_for_each(validate_geo_polygon)
        }
        Geometry::GeometryCollection(collection) => {
            validate_nonempty(&collection.0, "GeometryCollection GeoJSON vuota")?;
            collection.0.iter().try_for_each(validate_geo_geometry)
        }
        _ => Err(format_error(
            "geometria con Z/M non rappresentabile in GeoJSON 2D",
        )),
    }
}

fn validate_xy(x: f64, y: f64) -> Result<()> {
    if !x.is_finite() || !y.is_finite() {
        return Err(format_error(
            "coordinata non finita non rappresentabile in GeoJSON",
        ));
    }
    Ok(())
}

fn validate_geo_line(line: &geo_types::LineString<f64>) -> Result<()> {
    validate_nonempty(&line.0, "LineString GeoJSON vuota")?;
    line.0
        .iter()
        .try_for_each(|coordinate| validate_xy(coordinate.x, coordinate.y))
}

fn validate_geo_polygon(polygon: &geo_types::Polygon<f64>) -> Result<()> {
    validate_geo_line(polygon.exterior())?;
    polygon.interiors().iter().try_for_each(validate_geo_line)
}

fn write_geo_geojson_unchecked<W: Write>(
    writer: &mut W,
    geometry: &geo_types::Geometry<f64>,
) -> Result<()> {
    use geo_types::Geometry;

    match geometry {
        Geometry::Point(point) => {
            writer.write_all(b"{\"type\":\"Point\",\"coordinates\":")?;
            write_position(writer, point.x(), point.y())?;
            writer.write_all(b"}")?;
        }
        Geometry::LineString(line) => {
            writer.write_all(b"{\"type\":\"LineString\",\"coordinates\":")?;
            write_line(writer, line)?;
            writer.write_all(b"}")?;
        }
        Geometry::Polygon(polygon) => {
            writer.write_all(b"{\"type\":\"Polygon\",\"coordinates\":")?;
            write_polygon(writer, polygon)?;
            writer.write_all(b"}")?;
        }
        Geometry::MultiPoint(points) => {
            writer.write_all(b"{\"type\":\"MultiPoint\",\"coordinates\":[")?;
            for (index, point) in points.0.iter().enumerate() {
                write_separator(writer, index)?;
                write_position(writer, point.x(), point.y())?;
            }
            writer.write_all(b"]}")?;
        }
        Geometry::MultiLineString(lines) => {
            writer.write_all(b"{\"type\":\"MultiLineString\",\"coordinates\":[")?;
            for (index, line) in lines.0.iter().enumerate() {
                write_separator(writer, index)?;
                write_line(writer, line)?;
            }
            writer.write_all(b"]}")?;
        }
        Geometry::MultiPolygon(polygons) => {
            writer.write_all(b"{\"type\":\"MultiPolygon\",\"coordinates\":[")?;
            for (index, polygon) in polygons.0.iter().enumerate() {
                write_separator(writer, index)?;
                write_polygon(writer, polygon)?;
            }
            writer.write_all(b"]}")?;
        }
        Geometry::GeometryCollection(collection) => {
            writer.write_all(b"{\"type\":\"GeometryCollection\",\"geometries\":[")?;
            for (index, geometry) in collection.0.iter().enumerate() {
                write_separator(writer, index)?;
                write_geo_geojson_unchecked(writer, geometry)?;
            }
            writer.write_all(b"]}")?;
        }
        _ => {
            return Err(format_error(
                "geometria con Z/M non rappresentabile in GeoJSON 2D",
            ))
        }
    }
    Ok(())
}

fn write_position<W: Write>(writer: &mut W, x: f64, y: f64) -> Result<()> {
    // Ryu produce numeri JSON round-trippabili anche per gli estremi di f64.
    writer.write_all(b"[")?;
    serde_json::to_writer(&mut *writer, &x).map_err(|error| format_error(error.to_string()))?;
    writer.write_all(b",")?;
    serde_json::to_writer(&mut *writer, &y).map_err(|error| format_error(error.to_string()))?;
    writer.write_all(b"]")?;
    Ok(())
}

fn write_line<W: Write>(writer: &mut W, line: &geo_types::LineString<f64>) -> Result<()> {
    writer.write_all(b"[")?;
    for (index, coordinate) in line.0.iter().enumerate() {
        write_separator(writer, index)?;
        write_position(writer, coordinate.x, coordinate.y)?;
    }
    writer.write_all(b"]")?;
    Ok(())
}

fn write_polygon<W: Write>(writer: &mut W, polygon: &geo_types::Polygon<f64>) -> Result<()> {
    writer.write_all(b"[")?;
    write_line(writer, polygon.exterior())?;
    for ring in polygon.interiors() {
        writer.write_all(b",")?;
        write_line(writer, ring)?;
    }
    writer.write_all(b"]")?;
    Ok(())
}

fn write_separator<W: Write>(writer: &mut W, index: usize) -> Result<()> {
    if index > 0 {
        writer.write_all(b",")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use geo_types::{
        Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
        Polygon,
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
            Geometry::GeometryCollection(GeometryCollection(vec![Geometry::Polygon(
                Polygon::new(LineString::new(Vec::new()), Vec::new()),
            )])),
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
}
