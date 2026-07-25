//! Codec WKB in-tree condiviso: geo_types::Geometry <-> byte, i 7 tipi standard.
//! Scrittura 2D little-endian; lettura entrambi gli endian, Z/M ignorate.
//! Ogni conteggio è validato contro i byte residui e contro i limiti
//! (componenti totali, profondità): input ostile non forza allocazioni enormi.
//! Portato da plenora-geoparquet-tools, adattato all'errore del core.

use geo_types::{
    Coord, Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon,
};

use crate::error::{PlenoraError, Result};
use crate::limits::WkbLimits;

fn wkb_err(m: impl Into<String>) -> PlenoraError {
    PlenoraError::Wkb(m.into())
}

// --- scrittura (little-endian, 2D) ----------------------------------------

fn header(out: &mut Vec<u8>, code: u32) {
    out.push(1);
    out.extend_from_slice(&code.to_le_bytes());
}

fn put_ring(out: &mut Vec<u8>, ring: &LineString<f64>) {
    out.extend_from_slice(&(ring.0.len() as u32).to_le_bytes());
    for c in &ring.0 {
        out.extend_from_slice(&c.x.to_le_bytes());
        out.extend_from_slice(&c.y.to_le_bytes());
    }
}

fn put_polygon(out: &mut Vec<u8>, poly: &Polygon<f64>) {
    out.extend_from_slice(&((1 + poly.interiors().len()) as u32).to_le_bytes());
    put_ring(out, poly.exterior());
    for ring in poly.interiors() {
        put_ring(out, ring);
    }
}

fn write_geom(out: &mut Vec<u8>, geom: &Geometry<f64>) -> Result<()> {
    match geom {
        Geometry::Point(p) => {
            header(out, 1);
            out.extend_from_slice(&p.x().to_le_bytes());
            out.extend_from_slice(&p.y().to_le_bytes());
        }
        Geometry::LineString(ls) => {
            header(out, 2);
            put_ring(out, ls);
        }
        Geometry::Polygon(poly) => {
            header(out, 3);
            put_polygon(out, poly);
        }
        Geometry::MultiPoint(mp) => {
            header(out, 4);
            out.extend_from_slice(&(mp.0.len() as u32).to_le_bytes());
            for p in &mp.0 {
                header(out, 1);
                out.extend_from_slice(&p.x().to_le_bytes());
                out.extend_from_slice(&p.y().to_le_bytes());
            }
        }
        Geometry::MultiLineString(mls) => {
            header(out, 5);
            out.extend_from_slice(&(mls.0.len() as u32).to_le_bytes());
            for ls in &mls.0 {
                header(out, 2);
                put_ring(out, ls);
            }
        }
        Geometry::MultiPolygon(mp) => {
            header(out, 6);
            out.extend_from_slice(&(mp.0.len() as u32).to_le_bytes());
            for poly in &mp.0 {
                header(out, 3);
                put_polygon(out, poly);
            }
        }
        Geometry::GeometryCollection(gc) => {
            header(out, 7);
            out.extend_from_slice(&(gc.0.len() as u32).to_le_bytes());
            for g in &gc.0 {
                write_geom(out, g)?;
            }
        }
        _ => return Err(wkb_err("geometria non rappresentabile in WKB standard")),
    }
    Ok(())
}

/// Serializza in un buffer riusabile (lo **svuota** prima di scrivere). Evita
/// un'allocazione per riga nei loop di lettura streaming: il chiamante tiene un
/// solo `Vec` e ne appende il contenuto a un `BinaryBuilder` Arrow a ogni giro.
pub fn to_wkb_into(geom: &Geometry<f64>, out: &mut Vec<u8>) -> Result<()> {
    #[cfg(feature = "metrics")]
    crate::metrics::inc_encode();
    out.clear();
    write_geom(out, geom)
}

/// Serializza una geometria in WKB 2D little-endian (alloca un `Vec` nuovo).
/// Per i loop caldi preferire [`to_wkb_into`] con un buffer riusato.
pub fn to_wkb(geom: &Geometry<f64>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    to_wkb_into(geom, &mut out)?;
    Ok(out)
}

// --- lettura ---------------------------------------------------------------

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    components_left: usize,
    max_depth: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(wkb_err("WKB troncato"));
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn byte_order(&mut self) -> Result<bool> {
        match self.take(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            o => Err(wkb_err(format!("byte-order WKB non valido: {o}"))),
        }
    }
    fn u32(&mut self, le: bool) -> Result<u32> {
        let b: [u8; 4] = self.take(4)?.try_into().expect("4");
        Ok(if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }
    fn f64(&mut self, le: bool) -> Result<f64> {
        let b: [u8; 8] = self.take(8)?.try_into().expect("8");
        Ok(if le {
            f64::from_le_bytes(b)
        } else {
            f64::from_be_bytes(b)
        })
    }
    fn ensure(&self, count: u32, unit: usize) -> Result<usize> {
        let count = count as usize;
        match count.checked_mul(unit) {
            Some(need) if need <= self.remaining() => Ok(count),
            _ => Err(wkb_err("conteggio WKB oltre i byte disponibili")),
        }
    }
    fn charge_component(&mut self) -> Result<()> {
        if self.components_left == 0 {
            return Err(wkb_err("troppi componenti nella geometria"));
        }
        self.components_left -= 1;
        Ok(())
    }
}

fn decode_type(raw: u32) -> (u32, bool, bool) {
    if raw & 0xE000_0000 != 0 {
        return (raw & 0xFF, raw & 0x8000_0000 != 0, raw & 0x4000_0000 != 0);
    }
    let base = raw % 1000;
    let dim = raw / 1000;
    (base, dim == 1 || dim == 3, dim == 2 || dim == 3)
}

fn read_coord(r: &mut Reader, le: bool, z: bool, m: bool) -> Result<Coord<f64>> {
    r.charge_component()?;
    let x = r.f64(le)?;
    let y = r.f64(le)?;
    if z {
        r.f64(le)?;
    }
    if m {
        r.f64(le)?;
    }
    Ok(Coord { x, y })
}

fn coord_unit(z: bool, m: bool) -> usize {
    16 + if z { 8 } else { 0 } + if m { 8 } else { 0 }
}

fn read_ring(r: &mut Reader, le: bool, z: bool, m: bool) -> Result<LineString<f64>> {
    let count = r.u32(le)?;
    let n = r.ensure(count, coord_unit(z, m))?;
    let mut coords = Vec::with_capacity(n);
    for _ in 0..n {
        coords.push(read_coord(r, le, z, m)?);
    }
    Ok(LineString(coords))
}

fn read_polygon(r: &mut Reader, le: bool, z: bool, m: bool) -> Result<Polygon<f64>> {
    let count = r.u32(le)?;
    let nr = r.ensure(count, 4)?;
    let mut rings = Vec::with_capacity(nr);
    for _ in 0..nr {
        rings.push(read_ring(r, le, z, m)?);
    }
    if rings.is_empty() {
        return Ok(Polygon::new(LineString(Vec::new()), Vec::new()));
    }
    let ext = rings.remove(0);
    Ok(Polygon::new(ext, rings))
}

fn read_geom(r: &mut Reader, depth: usize) -> Result<Geometry<f64>> {
    if depth > r.max_depth {
        return Err(wkb_err("WKB annidato troppo in profondità"));
    }
    let le = r.byte_order()?;
    let (base, z, m) = decode_type(r.u32(le)?);
    match base {
        1 => Ok(Geometry::Point(Point(read_coord(r, le, z, m)?))),
        2 => Ok(Geometry::LineString(read_ring(r, le, z, m)?)),
        3 => Ok(Geometry::Polygon(read_polygon(r, le, z, m)?)),
        4 => {
            let count = r.u32(le)?;
            let n = r.ensure(count, 21)?;
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                match read_geom(r, depth + 1)? {
                    Geometry::Point(p) => v.push(p),
                    _ => return Err(wkb_err("MultiPoint con membro non-Point")),
                }
            }
            Ok(Geometry::MultiPoint(MultiPoint(v)))
        }
        5 => {
            let count = r.u32(le)?;
            let n = r.ensure(count, 9)?;
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                match read_geom(r, depth + 1)? {
                    Geometry::LineString(ls) => v.push(ls),
                    _ => return Err(wkb_err("MultiLineString con membro non-LineString")),
                }
            }
            Ok(Geometry::MultiLineString(MultiLineString(v)))
        }
        6 => {
            let count = r.u32(le)?;
            let n = r.ensure(count, 9)?;
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                match read_geom(r, depth + 1)? {
                    Geometry::Polygon(p) => v.push(p),
                    _ => return Err(wkb_err("MultiPolygon con membro non-Polygon")),
                }
            }
            Ok(Geometry::MultiPolygon(MultiPolygon(v)))
        }
        7 => {
            let count = r.u32(le)?;
            let n = r.ensure(count, 9)?;
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(read_geom(r, depth + 1)?);
            }
            Ok(Geometry::GeometryCollection(GeometryCollection(v)))
        }
        other => Err(wkb_err(format!("tipo WKB non supportato: {other}"))),
    }
}

/// Deserializza una geometria WKB (2D; Z/M ignorate), con guardie sui limiti.
pub fn from_wkb(bytes: &[u8], limits: &WkbLimits) -> Result<Geometry<f64>> {
    #[cfg(feature = "metrics")]
    crate::metrics::inc_decode();
    if bytes.len() > limits.max_cell_bytes {
        return Err(wkb_err("cella WKB oltre il limite di byte"));
    }
    let mut reader = Reader {
        bytes,
        pos: 0,
        components_left: limits.max_components,
        max_depth: limits.max_depth,
    };
    read_geom(&mut reader, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(g: Geometry<f64>) {
        let bytes = to_wkb(&g).unwrap();
        let back = from_wkb(&bytes, &WkbLimits::default()).unwrap();
        assert_eq!(g, back);
    }

    #[test]
    fn roundtrips_every_type() {
        roundtrip(Geometry::Point(Point::new(1.0, 2.0)));
        roundtrip(Geometry::LineString(LineString(vec![
            Coord { x: 0.0, y: 0.0 },
            Coord { x: 1.0, y: 1.0 },
        ])));
        let poly = Polygon::new(
            LineString(vec![
                Coord { x: 0.0, y: 0.0 },
                Coord { x: 4.0, y: 0.0 },
                Coord { x: 4.0, y: 4.0 },
                Coord { x: 0.0, y: 0.0 },
            ]),
            vec![],
        );
        roundtrip(Geometry::Polygon(poly.clone()));
        roundtrip(Geometry::MultiPoint(MultiPoint(vec![Point::new(0.0, 0.0)])));
        roundtrip(Geometry::MultiLineString(MultiLineString(vec![
            LineString(vec![Coord { x: 0.0, y: 0.0 }, Coord { x: 1.0, y: 1.0 }]),
        ])));
        roundtrip(Geometry::MultiPolygon(MultiPolygon(vec![poly])));
        roundtrip(Geometry::GeometryCollection(GeometryCollection(vec![
            Geometry::Point(Point::new(7.0, 8.0)),
        ])));
    }

    #[test]
    fn hostile_input_fails_closed() {
        assert!(from_wkb(&[], &WkbLimits::default()).is_err());
        let mut evil = vec![1u8, 2, 0, 0, 0];
        evil.extend_from_slice(&1_000_000_000u32.to_le_bytes());
        assert!(from_wkb(&evil, &WkbLimits::default()).is_err());
        // limite componenti
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
}
