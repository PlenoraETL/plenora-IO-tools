#![no_main]
//! Coverage-guided su `from_wkb`: nessun panic su byte arbitrari; se accetta,
//! il round-trip WKB dev'essere idempotente e lo scanner bbox non deve mai
//! sotto-stimare (invariante dello spatial pruning).
use libfuzzer_sys::fuzz_target;

use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{
    decode_wkb, encode_wkb, from_wkb, to_wkb, WkbFlavor, WkbGeometry, WkbValue,
};

fn lossless_finite(geometry: &WkbGeometry) -> bool {
    let coordinate = |coordinate: &plenora_io_model::wkb::WkbCoordinate| {
        coordinate.x.is_finite()
            && coordinate.y.is_finite()
            && coordinate.z.is_none_or(f64::is_finite)
            && coordinate.m.is_none_or(f64::is_finite)
    };
    match &geometry.value {
        WkbValue::Point(value) => coordinate(value),
        WkbValue::LineString(values) => values.iter().all(coordinate),
        WkbValue::Polygon(rings) => rings.iter().flatten().all(coordinate),
        WkbValue::MultiPoint(values)
        | WkbValue::MultiLineString(values)
        | WkbValue::MultiPolygon(values)
        | WkbValue::GeometryCollection(values) => values.iter().all(lossless_finite),
    }
}

fn coords(g: &geo_types::Geometry<f64>, out: &mut Vec<(f64, f64)>) {
    use geo_types::Geometry::*;
    match g {
        Point(p) => out.push((p.x(), p.y())),
        LineString(ls) => out.extend(ls.0.iter().map(|c| (c.x, c.y))),
        Polygon(pl) => {
            out.extend(pl.exterior().0.iter().map(|c| (c.x, c.y)));
            for r in pl.interiors() {
                out.extend(r.0.iter().map(|c| (c.x, c.y)));
            }
        }
        MultiPoint(mp) => out.extend(mp.0.iter().map(|p| (p.x(), p.y()))),
        MultiLineString(ml) => {
            for ls in &ml.0 {
                out.extend(ls.0.iter().map(|c| (c.x, c.y)));
            }
        }
        MultiPolygon(mp) => {
            for pl in &mp.0 {
                out.extend(pl.exterior().0.iter().map(|c| (c.x, c.y)));
                for r in pl.interiors() {
                    out.extend(r.0.iter().map(|c| (c.x, c.y)));
                }
            }
        }
        GeometryCollection(gc) => {
            for gg in &gc.0 {
                coords(gg, out);
            }
        }
        _ => {}
    }
}

fuzz_target!(|data: &[u8]| {
    let lim = WkbLimits::default();
    if let Ok(lossless) = decode_wkb(data, &lim) {
        let encoded =
            encode_wkb(&lossless, WkbFlavor::Ewkb).expect("AST WKB valido serializzabile");
        let decoded =
            decode_wkb(&encoded, &lim).expect("EWKB prodotto dal core deve essere valido");
        assert_eq!(lossless.geometry_type(), decoded.geometry_type());
        assert_eq!(lossless.dimensions, decoded.dimensions);
        assert_eq!(lossless.srid, decoded.srid);
        if lossless_finite(&lossless) {
            assert_eq!(lossless, decoded, "round-trip lossless Z/M/SRID");
        }
    }
    let g = match from_wkb(data, &lim) {
        Ok(g) => g,
        Err(_) => return,
    };
    // Idempotenza (salta con coordinate non finite: NaN != NaN).
    let mut cs = Vec::new();
    coords(&g, &mut cs);
    let finite = cs.iter().all(|(x, y)| x.is_finite() && y.is_finite());

    // Il limite di cella deve prevalere anche quando il prefisso contiene una
    // geometria valida: nessuna accettazione parziale oltre il budget.
    if !data.is_empty() {
        let byte_tight = WkbLimits {
            max_cell_bytes: data.len() - 1,
            ..lim
        };
        assert!(
            from_wkb(data, &byte_tight).is_err(),
            "max_cell_bytes non applicato"
        );
    }
    // Ogni geometria con coordinate deve consumare almeno un componente.
    if !cs.is_empty() {
        let component_tight = WkbLimits {
            max_components: 0,
            ..lim
        };
        assert!(
            from_wkb(data, &component_tight).is_err(),
            "max_components non applicato"
        );
    }

    let enc = to_wkb(&g).expect("to_wkb dopo from_wkb OK deve riuscire");
    let g2 = from_wkb(&enc, &lim).expect("re-decode del nostro WKB deve riuscire");
    if finite {
        assert_eq!(g, g2, "round-trip WKB non idempotente");
    }
    // Bbox scanner: non deve mai sotto-stimare.
    if let Some(bb) = driver_geoparquet::wkb_bbox(data) {
        let fin: Vec<_> = cs
            .iter()
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .collect();
        if !fin.is_empty() {
            let minx = fin.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min);
            let miny = fin.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
            let maxx = fin
                .iter()
                .map(|(x, _)| *x)
                .fold(f64::NEG_INFINITY, f64::max);
            let maxy = fin
                .iter()
                .map(|(_, y)| *y)
                .fold(f64::NEG_INFINITY, f64::max);
            let e = 1e-6 * (1.0 + minx.abs() + miny.abs() + maxx.abs() + maxy.abs());
            assert!(
                bb[0] <= minx + e && bb[1] <= miny + e && bb[2] >= maxx - e && bb[3] >= maxy - e,
                "wkb_bbox SOTTO-stima: {bb:?} vs [{minx},{miny},{maxx},{maxy}]"
            );
        }
    }
});
