#![no_main]
//! Coverage-guided sull'adattatore WKT dimensionale condiviso da CSV/XLSX.
use libfuzzer_sys::fuzz_target;

use driver_common::wkt_lossless::{format_wkt, parse_wkt_bounded};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{decode_wkb, encode_wkb, WkbFlavor, WkbGeometry, WkbValue};

/// WKT con i tetti predefiniti, in un posto solo.
///
/// Il target non ha opzioni da cui prendere una quota: dichiara la predefinita
/// una volta. I tetti li esercita comunque, perche' l'analisi li applica
/// **durante** il parse e il fuzzer arriva a superarli.
fn analizza(testo: &str) -> plenora_io_model::Result<WkbGeometry> {
    parse_wkt_bounded(testo, &WkbLimits::default())
}

fn all_finite(geometry: &WkbGeometry) -> bool {
    let coordinate = |coordinate: &plenora_io_model::wkb::WkbCoordinate| {
        coordinate.x.is_finite()
            && coordinate.y.is_finite()
            && coordinate.z.is_none_or(f64::is_finite)
            && coordinate.m.is_none_or(f64::is_finite)
    };
    match &geometry.value {
        WkbValue::Point(value) => coordinate(value),
        WkbValue::LineString(values) | WkbValue::CircularString(values) => {
            values.iter().all(coordinate)
        }
        WkbValue::Polygon(rings) | WkbValue::Triangle(rings) => {
            rings.iter().flatten().all(coordinate)
        }
        WkbValue::MultiPoint(values)
        | WkbValue::MultiLineString(values)
        | WkbValue::MultiPolygon(values)
        | WkbValue::GeometryCollection(values)
        | WkbValue::CompoundCurve(values)
        | WkbValue::CurvePolygon(values)
        | WkbValue::MultiCurve(values)
        | WkbValue::MultiSurface(values)
        | WkbValue::PolyhedralSurface(values)
        | WkbValue::Tin(values) => values.iter().all(all_finite),
    }
}

fuzz_target!(|data: &[u8]| {
    let text = match std::str::from_utf8(data) {
        Ok(text) => text,
        Err(_) => return,
    };
    if let Ok(geometry) = analizza(text) {
        let formatted = format_wkt(&geometry).expect("WKT accettato deve essere serializzabile");
        let reparsed = analizza(&formatted).expect("il nostro WKT deve essere rileggibile");
        let encoded =
            encode_wkb(&geometry, WkbFlavor::Iso).expect("WKT accettato deve produrre WKB");
        let decoded = decode_wkb(&encoded, &WkbLimits::default())
            .expect("il nostro WKB deve essere rileggibile");
        assert_eq!(geometry.geometry_type(), reparsed.geometry_type());
        assert_eq!(geometry.dimensions, reparsed.dimensions);
        assert_eq!(geometry.geometry_type(), decoded.geometry_type());
        assert_eq!(geometry.dimensions, decoded.dimensions);
        if all_finite(&geometry) {
            assert_eq!(geometry, reparsed, "round-trip WKT dimensionale");
            assert_eq!(geometry, decoded, "round-trip WKT → WKB");
        }
    }
});
