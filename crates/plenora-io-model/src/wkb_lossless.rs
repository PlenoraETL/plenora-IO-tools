use geo_types::{
    Coord, Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon,
};

use crate::contract::{CoordinateDimensions, GeometryType};
use crate::error::{PlenoraIoError, Result};
use crate::limits::WkbLimits;

fn error(message: impl Into<String>) -> PlenoraIoError {
    PlenoraIoError::Wkb(message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WkbFlavor {
    /// OGC/ISO WKB: dimensioni tramite offset 1000/2000/3000, nessun SRID.
    Iso,
    /// PostGIS EWKB: flag Z/M/SRID nel type word.
    Ewkb,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WkbCoordinate {
    pub x: f64,
    pub y: f64,
    pub z: Option<f64>,
    pub m: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WkbValue {
    Point(WkbCoordinate),
    LineString(Vec<WkbCoordinate>),
    Polygon(Vec<Vec<WkbCoordinate>>),
    MultiPoint(Vec<WkbGeometry>),
    MultiLineString(Vec<WkbGeometry>),
    MultiPolygon(Vec<WkbGeometry>),
    GeometryCollection(Vec<WkbGeometry>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct WkbGeometry {
    pub value: WkbValue,
    pub dimensions: CoordinateDimensions,
    pub srid: Option<i32>,
}

impl WkbGeometry {
    /// Adatta una geometria `geo-types` 2D all'AST WKB autoritativo.
    ///
    /// L'adattatore è intenzionalmente limitato ai sette tipi WKB classici:
    /// varianti `geo-types` come `Rect`, `Triangle` e `Line` richiedono una
    /// scelta di normalizzazione esplicita del chiamante.
    pub fn from_geo_xy(geometry: &Geometry<f64>) -> Result<Self> {
        fn coordinate(coordinate: Coord<f64>) -> WkbCoordinate {
            WkbCoordinate {
                x: coordinate.x,
                y: coordinate.y,
                z: None,
                m: None,
            }
        }

        fn line_string(line: &LineString<f64>) -> Vec<WkbCoordinate> {
            line.0.iter().copied().map(coordinate).collect()
        }

        fn polygon(polygon: &Polygon<f64>) -> Vec<Vec<WkbCoordinate>> {
            std::iter::once(polygon.exterior())
                .chain(polygon.interiors())
                .map(line_string)
                .collect()
        }

        fn child(value: WkbValue) -> WkbGeometry {
            WkbGeometry {
                value,
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            }
        }

        let value = match geometry {
            Geometry::Point(point) => WkbValue::Point(coordinate(point.0)),
            Geometry::LineString(line) => WkbValue::LineString(line_string(line)),
            Geometry::Polygon(value) => WkbValue::Polygon(polygon(value)),
            Geometry::MultiPoint(values) => WkbValue::MultiPoint(
                values
                    .iter()
                    .map(|point| child(WkbValue::Point(coordinate(point.0))))
                    .collect(),
            ),
            Geometry::MultiLineString(values) => WkbValue::MultiLineString(
                values
                    .iter()
                    .map(|line| child(WkbValue::LineString(line_string(line))))
                    .collect(),
            ),
            Geometry::MultiPolygon(values) => WkbValue::MultiPolygon(
                values
                    .iter()
                    .map(|value| child(WkbValue::Polygon(polygon(value))))
                    .collect(),
            ),
            Geometry::GeometryCollection(values) => WkbValue::GeometryCollection(
                values
                    .iter()
                    .map(WkbGeometry::from_geo_xy)
                    .collect::<Result<_>>()?,
            ),
            _ => return Err(error("geometria non rappresentabile in WKB standard")),
        };
        Ok(Self {
            value,
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        })
    }

    pub fn geometry_type(&self) -> GeometryType {
        match &self.value {
            WkbValue::Point(_) => GeometryType::Point,
            WkbValue::LineString(_) => GeometryType::LineString,
            WkbValue::Polygon(_) => GeometryType::Polygon,
            WkbValue::MultiPoint(_) => GeometryType::MultiPoint,
            WkbValue::MultiLineString(_) => GeometryType::MultiLineString,
            WkbValue::MultiPolygon(_) => GeometryType::MultiPolygon,
            WkbValue::GeometryCollection(_) => GeometryType::GeometryCollection,
        }
    }

    /// Proietta intenzionalmente su XY per gli adattatori v1 basati su
    /// `geo-types`. Per conservare Z/M usare l'AST o i byte originali.
    pub fn to_geo_xy(&self) -> Result<Geometry<f64>> {
        fn coord(c: WkbCoordinate) -> Coord<f64> {
            Coord { x: c.x, y: c.y }
        }
        fn point(value: &WkbGeometry) -> Result<Point<f64>> {
            match &value.value {
                WkbValue::Point(c) => Ok(Point(coord(*c))),
                _ => Err(error("MultiPoint con membro non-Point")),
            }
        }
        fn line(value: &WkbGeometry) -> Result<LineString<f64>> {
            match &value.value {
                WkbValue::LineString(coords) => {
                    Ok(LineString(coords.iter().copied().map(coord).collect()))
                }
                _ => Err(error("MultiLineString con membro non-LineString")),
            }
        }
        fn polygon(value: &WkbGeometry) -> Result<Polygon<f64>> {
            match &value.value {
                WkbValue::Polygon(rings) => {
                    let mut rings = rings
                        .iter()
                        .map(|ring| LineString(ring.iter().copied().map(coord).collect()))
                        .collect::<Vec<_>>();
                    if rings.is_empty() {
                        Ok(Polygon::new(LineString(Vec::new()), Vec::new()))
                    } else {
                        let exterior = rings.remove(0);
                        Ok(Polygon::new(exterior, rings))
                    }
                }
                _ => Err(error("MultiPolygon con membro non-Polygon")),
            }
        }

        Ok(match &self.value {
            WkbValue::Point(c) => Geometry::Point(Point(coord(*c))),
            WkbValue::LineString(coords) => {
                Geometry::LineString(LineString(coords.iter().copied().map(coord).collect()))
            }
            WkbValue::Polygon(_) => Geometry::Polygon(polygon(self)?),
            WkbValue::MultiPoint(values) => {
                Geometry::MultiPoint(MultiPoint(values.iter().map(point).collect::<Result<_>>()?))
            }
            WkbValue::MultiLineString(values) => Geometry::MultiLineString(MultiLineString(
                values.iter().map(line).collect::<Result<_>>()?,
            )),
            WkbValue::MultiPolygon(values) => Geometry::MultiPolygon(MultiPolygon(
                values.iter().map(polygon).collect::<Result<_>>()?,
            )),
            WkbValue::GeometryCollection(values) => {
                Geometry::GeometryCollection(GeometryCollection(
                    values
                        .iter()
                        .map(WkbGeometry::to_geo_xy)
                        .collect::<Result<_>>()?,
                ))
            }
        })
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    components_left: usize,
    max_depth: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(count)
            .ok_or_else(|| error("overflow nella posizione WKB"))?;
        let bytes = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| error("WKB troncato"))?;
        self.pos = end;
        Ok(bytes)
    }

    fn byte_order(&mut self) -> Result<bool> {
        match self
            .take(1)?
            .first()
            .copied()
            .ok_or_else(|| error("WKB senza byte-order"))?
        {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(error(format!("byte-order WKB non valido: {other}"))),
        }
    }

    fn u32(&mut self, little_endian: bool) -> Result<u32> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| error("WKB troncato durante la lettura di u32"))?;
        Ok(if little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    }

    fn f64(&mut self, little_endian: bool) -> Result<f64> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| error("WKB troncato durante la lettura di f64"))?;
        Ok(if little_endian {
            f64::from_le_bytes(bytes)
        } else {
            f64::from_be_bytes(bytes)
        })
    }

    fn ensure_count(&self, count: u32, unit: usize) -> Result<usize> {
        let count = count as usize;
        match count.checked_mul(unit) {
            Some(required) if required <= self.remaining() => Ok(count),
            _ => Err(error("conteggio WKB oltre i byte disponibili")),
        }
    }

    fn charge_coordinate(&mut self) -> Result<()> {
        if self.components_left == 0 {
            return Err(error("troppi componenti nella geometria"));
        }
        self.components_left -= 1;
        Ok(())
    }
}

fn dimension_flags(dimensions: CoordinateDimensions) -> Result<(bool, bool)> {
    match dimensions {
        CoordinateDimensions::Xy => Ok((false, false)),
        CoordinateDimensions::Xyz => Ok((true, false)),
        CoordinateDimensions::Xym => Ok((false, true)),
        CoordinateDimensions::Xyzm => Ok((true, true)),
        CoordinateDimensions::Unknown => Err(error("dimensionalità WKB ignota non serializzabile")),
    }
}

fn decode_type(raw: u32) -> Result<(u32, CoordinateDimensions, bool)> {
    if raw & 0xE000_0000 != 0 {
        let base = raw & 0x1FFF_FFFF;
        let dimensions = match (raw & 0x8000_0000 != 0, raw & 0x4000_0000 != 0) {
            (false, false) => CoordinateDimensions::Xy,
            (true, false) => CoordinateDimensions::Xyz,
            (false, true) => CoordinateDimensions::Xym,
            (true, true) => CoordinateDimensions::Xyzm,
        };
        return Ok((base, dimensions, raw & 0x2000_0000 != 0));
    }

    let dimensions = match raw / 1000 {
        0 => CoordinateDimensions::Xy,
        1 => CoordinateDimensions::Xyz,
        2 => CoordinateDimensions::Xym,
        3 => CoordinateDimensions::Xyzm,
        dimension => {
            return Err(error(format!(
                "codice dimensionale WKB non valido: {dimension}"
            )))
        }
    };
    Ok((raw % 1000, dimensions, false))
}

fn coordinate_size(dimensions: CoordinateDimensions) -> Result<usize> {
    let (z, m) = dimension_flags(dimensions)?;
    Ok(16 + usize::from(z) * 8 + usize::from(m) * 8)
}

fn read_coordinate(
    reader: &mut Reader,
    little_endian: bool,
    dimensions: CoordinateDimensions,
) -> Result<WkbCoordinate> {
    reader.charge_coordinate()?;
    let (has_z, has_m) = dimension_flags(dimensions)?;
    Ok(WkbCoordinate {
        x: reader.f64(little_endian)?,
        y: reader.f64(little_endian)?,
        z: if has_z {
            Some(reader.f64(little_endian)?)
        } else {
            None
        },
        m: if has_m {
            Some(reader.f64(little_endian)?)
        } else {
            None
        },
    })
}

fn read_coordinates(
    reader: &mut Reader,
    little_endian: bool,
    dimensions: CoordinateDimensions,
) -> Result<Vec<WkbCoordinate>> {
    let count = reader.u32(little_endian)?;
    let count = reader.ensure_count(count, coordinate_size(dimensions)?)?;
    let mut coordinates = Vec::with_capacity(count);
    for _ in 0..count {
        coordinates.push(read_coordinate(reader, little_endian, dimensions)?);
    }
    Ok(coordinates)
}

fn read_geometry(reader: &mut Reader, depth: usize) -> Result<WkbGeometry> {
    if depth > reader.max_depth {
        return Err(error("WKB annidato troppo in profondità"));
    }
    let little_endian = reader.byte_order()?;
    let (base, dimensions, has_srid) = decode_type(reader.u32(little_endian)?)?;
    if !(1..=7).contains(&base) {
        return Err(error(format!("tipo WKB non supportato: {base}")));
    }
    let srid = if has_srid {
        Some(reader.u32(little_endian)? as i32)
    } else {
        None
    };

    let value = match base {
        1 => WkbValue::Point(read_coordinate(reader, little_endian, dimensions)?),
        2 => WkbValue::LineString(read_coordinates(reader, little_endian, dimensions)?),
        3 => {
            let ring_count = reader.u32(little_endian)?;
            let ring_count = reader.ensure_count(ring_count, 4)?;
            let mut rings = Vec::with_capacity(ring_count);
            for _ in 0..ring_count {
                rings.push(read_coordinates(reader, little_endian, dimensions)?);
            }
            WkbValue::Polygon(rings)
        }
        4..=7 => {
            let count = reader.u32(little_endian)?;
            let count = reader.ensure_count(count, 9)?;
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                values.push(read_geometry(reader, depth + 1)?);
            }
            match base {
                4 if values
                    .iter()
                    .all(|value| matches!(value.value, WkbValue::Point(_))) =>
                {
                    WkbValue::MultiPoint(values)
                }
                5 if values
                    .iter()
                    .all(|value| matches!(value.value, WkbValue::LineString(_))) =>
                {
                    WkbValue::MultiLineString(values)
                }
                6 if values
                    .iter()
                    .all(|value| matches!(value.value, WkbValue::Polygon(_))) =>
                {
                    WkbValue::MultiPolygon(values)
                }
                7 => WkbValue::GeometryCollection(values),
                4 => return Err(error("MultiPoint con membro non-Point")),
                5 => return Err(error("MultiLineString con membro non-LineString")),
                6 => return Err(error("MultiPolygon con membro non-Polygon")),
                aggregate_type => {
                    return Err(error(format!(
                        "tipo WKB aggregato non supportato: {aggregate_type}"
                    )))
                }
            }
        }
        unsupported_type => {
            return Err(error(format!(
                "tipo WKB non supportato: {unsupported_type}"
            )))
        }
    };

    Ok(WkbGeometry {
        value,
        dimensions,
        srid,
    })
}

/// Decodifica WKB ISO o EWKB senza perdere Z, M o SRID.
pub fn decode_wkb(bytes: &[u8], limits: &WkbLimits) -> Result<WkbGeometry> {
    #[cfg(feature = "metrics")]
    crate::metrics::inc_decode();
    if bytes.len() > limits.max_cell_bytes {
        return Err(error("cella WKB oltre il limite di byte"));
    }
    let mut reader = Reader {
        bytes,
        pos: 0,
        components_left: limits.max_components,
        max_depth: limits.max_depth,
    };
    let geometry = read_geometry(&mut reader, 0)?;
    if reader.pos != bytes.len() {
        return Err(error("byte residui dopo la geometria WKB"));
    }
    Ok(geometry)
}

fn count_u32(count: usize) -> Result<u32> {
    count
        .try_into()
        .map_err(|_| error("troppi componenti per il formato WKB"))
}

fn validate_coordinate(coordinate: &WkbCoordinate, dimensions: CoordinateDimensions) -> Result<()> {
    let valid = match dimensions {
        CoordinateDimensions::Xy => coordinate.z.is_none() && coordinate.m.is_none(),
        CoordinateDimensions::Xyz => coordinate.z.is_some() && coordinate.m.is_none(),
        CoordinateDimensions::Xym => coordinate.z.is_none() && coordinate.m.is_some(),
        CoordinateDimensions::Xyzm => coordinate.z.is_some() && coordinate.m.is_some(),
        CoordinateDimensions::Unknown => false,
    };
    if valid {
        Ok(())
    } else {
        Err(error(
            "ordinate non coerenti con la dimensionalità dichiarata",
        ))
    }
}

fn write_coordinate(
    output: &mut Vec<u8>,
    coordinate: &WkbCoordinate,
    dimensions: CoordinateDimensions,
) -> Result<()> {
    validate_coordinate(coordinate, dimensions)?;
    output.extend_from_slice(&coordinate.x.to_le_bytes());
    output.extend_from_slice(&coordinate.y.to_le_bytes());
    if let Some(z) = coordinate.z {
        output.extend_from_slice(&z.to_le_bytes());
    }
    if let Some(m) = coordinate.m {
        output.extend_from_slice(&m.to_le_bytes());
    }
    Ok(())
}

fn write_coordinates(
    output: &mut Vec<u8>,
    coordinates: &[WkbCoordinate],
    dimensions: CoordinateDimensions,
) -> Result<()> {
    output.extend_from_slice(&count_u32(coordinates.len())?.to_le_bytes());
    for coordinate in coordinates {
        write_coordinate(output, coordinate, dimensions)?;
    }
    Ok(())
}

fn base(value: &WkbValue) -> u32 {
    match value {
        WkbValue::Point(_) => 1,
        WkbValue::LineString(_) => 2,
        WkbValue::Polygon(_) => 3,
        WkbValue::MultiPoint(_) => 4,
        WkbValue::MultiLineString(_) => 5,
        WkbValue::MultiPolygon(_) => 6,
        WkbValue::GeometryCollection(_) => 7,
    }
}

fn write_geometry(output: &mut Vec<u8>, geometry: &WkbGeometry, flavor: WkbFlavor) -> Result<()> {
    let (z, m) = dimension_flags(geometry.dimensions)?;
    let type_code = match flavor {
        WkbFlavor::Iso => {
            if geometry.srid.is_some() {
                return Err(error("WKB ISO non può rappresentare un SRID"));
            }
            base(&geometry.value)
                + match (z, m) {
                    (false, false) => 0,
                    (true, false) => 1000,
                    (false, true) => 2000,
                    (true, true) => 3000,
                }
        }
        WkbFlavor::Ewkb => {
            base(&geometry.value)
                | if z { 0x8000_0000 } else { 0 }
                | if m { 0x4000_0000 } else { 0 }
                | if geometry.srid.is_some() {
                    0x2000_0000
                } else {
                    0
                }
        }
    };
    output.push(1);
    output.extend_from_slice(&type_code.to_le_bytes());
    if let Some(srid) = geometry.srid {
        output.extend_from_slice(&(srid as u32).to_le_bytes());
    }

    match &geometry.value {
        WkbValue::Point(coordinate) => write_coordinate(output, coordinate, geometry.dimensions)?,
        WkbValue::LineString(coordinates) => {
            write_coordinates(output, coordinates, geometry.dimensions)?
        }
        WkbValue::Polygon(rings) => {
            output.extend_from_slice(&count_u32(rings.len())?.to_le_bytes());
            for ring in rings {
                write_coordinates(output, ring, geometry.dimensions)?;
            }
        }
        WkbValue::MultiPoint(values)
        | WkbValue::MultiLineString(values)
        | WkbValue::MultiPolygon(values)
        | WkbValue::GeometryCollection(values) => {
            output.extend_from_slice(&count_u32(values.len())?.to_le_bytes());
            for value in values {
                write_geometry(output, value, flavor)?;
            }
        }
    }
    Ok(())
}

pub fn encode_wkb_into(
    geometry: &WkbGeometry,
    flavor: WkbFlavor,
    output: &mut Vec<u8>,
) -> Result<()> {
    #[cfg(feature = "metrics")]
    crate::metrics::inc_encode();
    output.clear();
    write_geometry(output, geometry, flavor)
}

pub fn encode_wkb(geometry: &WkbGeometry, flavor: WkbFlavor) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    encode_wkb_into(geometry, flavor, &mut output)?;
    Ok(output)
}
