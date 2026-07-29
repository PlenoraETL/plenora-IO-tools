//! plenora-fuzz — fuzzer a mutazione strutturata in **Rust stabile** (niente
//! nightly/clang: gira robusto e non presidiato per ore). Martella i kernel
//! critici con input mutati/casuali e verifica INVARIANTI forti; ogni input che
//! viola un'invariante o fa panic viene salvato come artefatto riproducibile.
//!
//! Target e invarianti:
//!  1. `from_wkb`  — non deve MAI andare in panic; se accetta, deve essere
//!     idempotente (`from_wkb(to_wkb(g)) == g`) e non allocare senza limiti.
//!  2. `wkb_bbox` (scanner raw-bytes) — non deve MAI SOTTO-stimare il bbox
//!     (invariante di correttezza dello spatial pruning: mai scartare in eccesso).
//!  3. `wkb_from_gj_value` — non panic; se produce WKB, dev'essere decodificabile.
//!  4. `write_geo_geojson` — round-trip: geo→JSON→geojson::Value→WKB→geo.
//!
//! Uso:
//!   plenora-fuzz                      # campagna (PLENORA_FUZZ_SECONDS, def 60)
//!   plenora-fuzz <file>               # replay: esegue i check su un artefatto
//!   plenora-fuzz --export-corpus DIR  # seed per i target cargo-fuzz
//! Env: PLENORA_FUZZ_SECONDS, PLENORA_FUZZ_SEED, PLENORA_FUZZ_OUT.

use std::collections::HashSet;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use driver_common::wkt_lossless::{format_wkt, parse_wkt};
use geo_types::Geometry;
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{
    decode_wkb, encode_wkb, from_wkb, inspect_wkb, to_wkb, WkbFlavor, WkbGeometry, WkbValue,
};

// --- PRNG deterministico (xorshift64) --------------------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    fn byte(&mut self) -> u8 {
        (self.next() & 0xff) as u8
    }
    fn f64(&mut self) -> f64 {
        match self.below(8) {
            0 => 0.0,
            1 => f64::NAN,
            2 => f64::INFINITY,
            3 => -0.0,
            4 => 1e308,
            5 => f64::from_bits(self.next()),
            6 => -1e-9,
            _ => (self.next() as i64 as f64) * 1e-3,
        }
    }
}

// --- corpus di semi validi -------------------------------------------------

fn raw_point_seed(le: bool, raw_type: u32, srid: Option<u32>, ordinates: &[f64]) -> Vec<u8> {
    let mut out = vec![u8::from(le)];
    if le {
        out.extend_from_slice(&raw_type.to_le_bytes());
        if let Some(srid) = srid {
            out.extend_from_slice(&srid.to_le_bytes());
        }
        for value in ordinates {
            out.extend_from_slice(&value.to_le_bytes());
        }
    } else {
        out.extend_from_slice(&raw_type.to_be_bytes());
        if let Some(srid) = srid {
            out.extend_from_slice(&srid.to_be_bytes());
        }
        for value in ordinates {
            out.extend_from_slice(&value.to_be_bytes());
        }
    }
    out
}

fn wkb_seeds() -> Vec<Vec<u8>> {
    use geo_types::{
        Coord, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon, Point,
        Polygon,
    };
    let c = |x: f64, y: f64| Coord { x, y };
    let ext = LineString(vec![c(0., 0.), c(4., 0.), c(4., 4.), c(0., 4.), c(0., 0.)]);
    let hole = LineString(vec![c(1., 1.), c(2., 1.), c(2., 2.), c(1., 1.)]);
    let gs: Vec<Geometry<f64>> = vec![
        Geometry::Point(Point::new(1., 2.)),
        Geometry::LineString(LineString(vec![c(0., 0.), c(1., 1.), c(2., 3.)])),
        Geometry::Polygon(Polygon::new(ext.clone(), vec![hole.clone()])),
        Geometry::MultiPoint(MultiPoint(vec![Point::new(0., 0.), Point::new(5., 5.)])),
        Geometry::MultiLineString(MultiLineString(vec![LineString(vec![
            c(0., 0.),
            c(9., 9.),
        ])])),
        Geometry::MultiPolygon(MultiPolygon(vec![Polygon::new(ext, vec![])])),
        Geometry::GeometryCollection(GeometryCollection(vec![
            Geometry::Point(Point::new(7., 8.)),
            Geometry::Polygon(Polygon::new(
                LineString(vec![c(0., 0.), c(1., 0.), c(1., 1.), c(0., 0.)]),
                vec![],
            )),
        ])),
    ];
    let mut seeds: Vec<Vec<u8>> = gs.iter().map(|g| to_wkb(g).unwrap()).collect();

    // Endian e codifiche dimensionali accettate dalla v1: Z/M vengono
    // consumate, poi normalizzate esplicitamente a XY.
    seeds.push(raw_point_seed(false, 1, None, &[1.0, 2.0]));
    seeds.push(raw_point_seed(true, 1001, None, &[1.0, 2.0, 3.0]));
    seeds.push(raw_point_seed(true, 2001, None, &[1.0, 2.0, 9.0]));
    seeds.push(raw_point_seed(true, 3001, None, &[1.0, 2.0, 3.0, 9.0]));
    seeds.push(raw_point_seed(true, 0x8000_0001, None, &[1.0, 2.0, 3.0]));
    seeds.push(raw_point_seed(true, 0x4000_0001, None, &[1.0, 2.0, 9.0]));
    seeds.push(raw_point_seed(
        true,
        0xC000_0001,
        None,
        &[1.0, 2.0, 3.0, 9.0],
    ));

    // EWKB con SRID: il decoder lossless deve conservarlo e l'adattatore XY
    // legacy deve consumarlo correttamente prima delle coordinate.
    seeds.push(raw_point_seed(true, 0x2000_0001, Some(4326), &[1.0, 2.0]));
    seeds.push(raw_point_seed(
        true,
        0xA000_0001,
        Some(4326),
        &[1.0, 2.0, 3.0],
    ));
    seeds
}

fn geojson_seeds() -> Vec<Vec<u8>> {
    [
        r#"{"type":"Point","coordinates":[1.0,2.0]}"#,
        r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,2]]}"#,
        r#"{"type":"Polygon","coordinates":[[[0,0],[4,0],[4,4],[0,0]],[[1,1],[2,1],[2,2],[1,1]]]}"#,
        r#"{"type":"MultiPoint","coordinates":[[0,0],[5,5]]}"#,
        r#"{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]]]}"#,
        r#"{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[7,8]}]}"#,
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
}

/// WKT dei 7 tipi e di tutte le dimensioni: percorso geometria condiviso di
/// CSV/XLSX (il parser di terze parti resta esposto a input ostile).
fn wkt_seeds() -> Vec<Vec<u8>> {
    [
        "POINT (1 2)",
        "POINT EMPTY",
        "POINT Z (1 2 3)",
        "POINT M (1 2 9)",
        "POINT ZM (1 2 3 9)",
        "POINT (1e308 -1e-308)",
        "LINESTRING (0 0, 1 1, 2 2)",
        "LINESTRING EMPTY",
        "POLYGON ((0 0, 4 0, 4 4, 0 0), (1 1, 2 1, 2 2, 1 1))",
        "MULTIPOINT ((0 0), (5 5))",
        "MULTILINESTRING ((0 0, 1 1), (2 2, 3 3))",
        "MULTIPOLYGON (((0 0, 1 0, 1 1, 0 0)))",
        "GEOMETRYCOLLECTION (POINT (7 8), LINESTRING (0 0, 1 1))",
        "POINT Z (1 2 3)",
        "LINESTRING M (0 0 10, 1 1 11)",
        "MULTIPOLYGON ZM (((0 0 1 10, 0 2 2 11, 2 0 3 12, 0 0 1 10)))",
        "GEOMETRYCOLLECTION Z (POINT Z (7 8 9), LINESTRING Z (0 0 0, 1 1 1))",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
}

/// FeatureCollection eterogenee: proprietà disomogenee, tipi misti, geometria
/// null e properties null — per stressare pass-1 + pass-2 (allineamento colonne).
fn fc_seeds() -> Vec<Vec<u8>> {
    [
        r#"{"type":"FeatureCollection","features":[]}"#,
        r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Point","coordinates":[1,2]},"properties":{"a":1,"b":"x","c":true}}]}"#,
        r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":null,"properties":{"a":2}},{"type":"Feature","geometry":{"type":"Point","coordinates":[3,3]},"properties":null},{"type":"Feature","geometry":{"type":"LineString","coordinates":[[0,0],[1,1]]},"properties":{"b":"y","d":9.5}}]}"#,
        r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]},"properties":{"n":-3,"arr":[1,2,3],"obj":{"k":1}}}]}"#,
        r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"MultiPoint","coordinates":[[0,0],[1,1]]},"properties":{"n":1}},{"type":"Feature","geometry":{"type":"MultiLineString","coordinates":[[[0,0],[1,1]],[[2,2],[3,3]]]},"properties":{"n":2}},{"type":"Feature","geometry":{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]]]},"properties":{"n":3}},{"type":"Feature","geometry":{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[7,8]}]},"properties":{"n":4}}]}"#,
        r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Point","coordinates":[1,2,3]},"properties":{"precise":0.12345678901234567,"huge":1.7976931348623157e308}}]}"#,
        r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Point","coordinates":[1,2]},"geometry":{"type":"Point","coordinates":[9,9]},"properties":{"c":1,"b":"x","c":true}}]}"#,
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
}

fn kml_seeds() -> Vec<Vec<u8>> {
    [
        r#"<?xml version="1.0"?><kml xmlns="http://www.opengis.net/kml/2.2"><Document/></kml>"#,
        r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Placemark><Point><coordinates>12.5,45.9</coordinates></Point></Placemark></kml>"#,
        r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Placemark><LineString><coordinates>0,0,10 1,1,20</coordinates></LineString></Placemark></kml>"#,
        r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Placemark><Polygon><outerBoundaryIs><LinearRing><coordinates>0,0,10 1,0,11 1,1,12 0,0,10</coordinates></LinearRing></outerBoundaryIs></Polygon></Placemark></kml>"#,
        r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Placemark><MultiGeometry><Point><coordinates>1,2,3</coordinates></Point><LineString><coordinates>0,0,0 1,1,1</coordinates></LineString></MultiGeometry></Placemark></kml>"#,
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
}

fn export_seed_set(root: &Path, target: &str, extension: &str, seeds: &[Vec<u8>]) {
    let dir = root.join(target);
    std::fs::create_dir_all(&dir).expect("creazione directory corpus");
    for (index, seed) in seeds.iter().enumerate() {
        let path = dir.join(format!("seed-{index:02}.{extension}"));
        std::fs::write(path, seed).expect("scrittura seed corpus");
    }
}

fn export_corpus(root: &Path) {
    let wkb = wkb_seeds();
    let fc = fc_seeds();
    let wkt = wkt_seeds();
    let kml = kml_seeds();
    export_seed_set(root, "from_wkb", "wkb", &wkb);
    export_seed_set(root, "shp_wkb", "wkb", &wkb);
    export_seed_set(root, "geojson_reader", "geojson", &fc);
    export_seed_set(root, "wkt_parse", "wkt", &wkt);
    export_seed_set(root, "kml_reader", "kml", &kml);
    println!(
        "corpus esportato in {}: WKB={} (core+SHP), GeoJSON={}, WKT={}, KML={}",
        root.display(),
        wkb.len(),
        fc.len(),
        wkt.len(),
        kml.len()
    );
}

fn replay_shared_corpus(root: &Path) -> Result<serde_json::Value, String> {
    let manifest_path = root.join("manifest.json");
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("lettura {}: {error}", manifest_path.display()))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text)
        .map_err(|error| format!("manifest condiviso non valido: {error}"))?;
    let cases = manifest
        .get("cases")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "manifest condiviso senza cases array".to_owned())?;
    let limits = WkbLimits::default();
    let mut observations = Vec::with_capacity(cases.len());
    let mut failures = Vec::new();

    for case in cases {
        let path = case
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "caso senza path".to_owned())?;
        let expectation = case
            .get("expectation")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("{path}: expectation assente"))?;
        let payload_path = root.join(path);
        let payload = std::fs::read(&payload_path)
            .map_err(|error| format!("lettura {}: {error}", payload_path.display()))?;
        match inspect_wkb(&payload, &limits) {
            Ok(inspection) => {
                let geometry_type = inspection.geometry_type.canonical_name();
                let dimensions = serde_json::to_value(inspection.dimensions)
                    .map_err(|error| format!("{path}: dimensioni non serializzabili: {error}"))?;
                let dimensions = dimensions
                    .as_str()
                    .ok_or_else(|| format!("{path}: dimensioni non stringa"))?;
                if expectation == "rejected" {
                    failures.push(format!("{path}: accettato ma atteso rejected"));
                }
                if expectation == "accepted" {
                    if case
                        .get("geometry_type")
                        .and_then(serde_json::Value::as_str)
                        != Some(geometry_type)
                    {
                        failures.push(format!("{path}: geometry_type divergente"));
                    }
                    if case.get("dimensions").and_then(serde_json::Value::as_str)
                        != Some(dimensions)
                    {
                        failures.push(format!("{path}: dimensions divergenti"));
                    }
                    if case.get("srid").and_then(serde_json::Value::as_i64)
                        != inspection.srid.map(i64::from)
                    {
                        failures.push(format!("{path}: SRID divergente"));
                    }
                }
                observations.push(serde_json::json!({
                    "path": path,
                    "accepted": true,
                    "geometry_type": geometry_type,
                    "dimensions": dimensions,
                    "srid": inspection.srid,
                    "nested_dimensions_coherent": inspection.nested_dimensions_coherent,
                    "contains_srid": inspection.contains_srid,
                }));
            }
            Err(error) => {
                if expectation == "accepted" {
                    failures.push(format!("{path}: rifiutato ma atteso accepted"));
                }
                observations.push(serde_json::json!({
                    "path": path,
                    "accepted": false,
                    "error_category": error.category,
                    "error_code": error.code,
                }));
            }
        }
    }

    if !failures.is_empty() {
        return Err(failures.join("\n"));
    }
    Ok(serde_json::json!({
        "component": "plenora-IO-tools",
        "revision": option_env!("GIT_COMMIT"),
        "cases": observations,
    }))
}

// --- mutazione di byte -----------------------------------------------------

fn mutate(rng: &mut Rng, seed: &[u8]) -> Vec<u8> {
    let mut v = seed.to_vec();
    let ops = 1 + rng.below(6);
    for _ in 0..ops {
        if v.is_empty() {
            v.push(rng.byte());
            continue;
        }
        match rng.below(7) {
            0 => {
                let i = rng.below(v.len());
                v[i] = rng.byte();
            }
            1 => {
                let i = rng.below(v.len());
                v[i] ^= 1 << rng.below(8);
            }
            2 => {
                let i = rng.below(v.len() + 1);
                v.insert(i, rng.byte());
            }
            3 => {
                if v.len() > 1 {
                    let i = rng.below(v.len());
                    v.remove(i);
                }
            }
            4 => {
                // Corrompe un u32 (spesso un length field) con un valore estremo.
                if v.len() >= 4 {
                    let i = rng.below(v.len() - 3);
                    let val = [0u32, 1, 2, 0xffff_ffff, 0x7fff_ffff, 0x8000_0000][rng.below(6)];
                    v[i..i + 4].copy_from_slice(&val.to_le_bytes());
                }
            }
            5 => {
                let t = rng.below(v.len());
                v.truncate(t);
            }
            _ => {
                // Duplica un tratto (fa crescere le strutture ripetute).
                if !v.is_empty() {
                    let i = rng.below(v.len());
                    let b = v[i];
                    v.insert(i, b);
                }
            }
        }
    }
    v
}

// --- geometrie: helper per le invarianti -----------------------------------

fn collect_coords(g: &Geometry<f64>, out: &mut Vec<(f64, f64)>) {
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
                collect_coords(gg, out);
            }
        }
        _ => {} // from_wkb non produce Line/Rect/Triangle
    }
}

fn all_finite(g: &Geometry<f64>) -> bool {
    let mut v = Vec::new();
    collect_coords(g, &mut v);
    v.iter().all(|(x, y)| x.is_finite() && y.is_finite())
}

fn lossless_finite(geometry: &WkbGeometry) -> bool {
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
        | WkbValue::Tin(values) => values.iter().all(lossless_finite),
    }
}

// --- i check (ognuno ritorna Err(descrizione) su violazione) ---------------

const LIM: WkbLimits = WkbLimits {
    max_cell_bytes: 1 << 20,
    max_components: 1 << 20,
    max_depth: 64,
};

/// Invarianti sul codec WKB e sullo scanner bbox, da byte grezzi.
fn check_wkb(data: &[u8]) -> Result<(), String> {
    let decoded_lossless = decode_wkb(data, &LIM);
    let inspected = inspect_wkb(data, &LIM);
    match (&decoded_lossless, &inspected) {
        (Ok(decoded), Ok(inspection)) => {
            if decoded.geometry_type() != inspection.geometry_type
                || decoded.dimensions != inspection.dimensions
                || decoded.srid != inspection.srid
            {
                return Err("visitor WKB e decoder divergono su tipo/dimensioni/SRID".to_owned());
            }
        }
        (Ok(_), Err(error)) => {
            return Err(format!(
                "visitor WKB rifiuta input accettato dal decoder: {error}"
            ));
        }
        (Err(error), Ok(_)) => {
            return Err(format!(
                "visitor WKB accetta input rifiutato dal decoder: {error}"
            ));
        }
        (Err(_), Err(_)) => {}
    }
    if let Ok(lossless) = decoded_lossless {
        let encoded = encode_wkb(&lossless, WkbFlavor::Ewkb)
            .map_err(|e| format!("encode lossless dopo decode OK fallisce: {e}"))?;
        let decoded = decode_wkb(&encoded, &LIM)
            .map_err(|e| format!("re-decode EWKB lossless fallisce: {e}"))?;
        if lossless.geometry_type() != decoded.geometry_type()
            || lossless.dimensions != decoded.dimensions
            || lossless.srid != decoded.srid
        {
            return Err("round-trip lossless altera tipo/dimensioni/SRID".to_owned());
        }
        if lossless_finite(&lossless) && lossless != decoded {
            return Err("round-trip lossless altera ordinate Z/M".to_owned());
        }
    }
    let g = match from_wkb(data, &LIM) {
        Ok(g) => g,
        Err(_) => return Ok(()), // rifiuto pulito: ok
    };
    // Idempotenza: re-encode + re-decode deve dare la stessa geometria
    // (salta l'uguaglianza se ci sono coordinate non finite: NaN != NaN).
    let enc = to_wkb(&g).map_err(|e| format!("to_wkb dopo from_wkb OK fallisce: {e}"))?;
    let g2 = from_wkb(&enc, &LIM).map_err(|e| format!("re-decode del nostro WKB fallisce: {e}"))?;
    if all_finite(&g) && g2 != g {
        return Err(format!("round-trip WKB non idempotente: {g:?} != {g2:?}"));
    }
    // Bbox scanner: non deve MAI sotto-stimare (spatial pruning conservativo).
    if let Some(bb) = driver_geoparquet::wkb_bbox(data) {
        let mut cs = Vec::new();
        collect_coords(&g, &mut cs);
        let finite: Vec<_> = cs
            .iter()
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .collect();
        if !finite.is_empty() {
            let minx = finite.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min);
            let miny = finite.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
            let maxx = finite
                .iter()
                .map(|(x, _)| *x)
                .fold(f64::NEG_INFINITY, f64::max);
            let maxy = finite
                .iter()
                .map(|(_, y)| *y)
                .fold(f64::NEG_INFINITY, f64::max);
            let e = 1e-6 * (1.0 + minx.abs() + miny.abs() + maxx.abs() + maxy.abs());
            if bb[0] > minx + e || bb[1] > miny + e || bb[2] < maxx - e || bb[3] < maxy - e {
                return Err(format!(
                    "wkb_bbox SOTTO-stima: bbox={bb:?} vero=[{minx},{miny},{maxx},{maxy}]"
                ));
            }
        }
    }
    // Round-trip attraverso le funzioni geometriche geojson dirette.
    if all_finite(&g) {
        geojson_geom_roundtrip(&g)?;
    }
    Ok(())
}

/// geo → write_geo_geojson → geojson::Geometry → wkb_from_gj_value → geo.
fn geojson_geom_roundtrip(g: &Geometry<f64>) -> Result<(), String> {
    let mut json: Vec<u8> = Vec::new();
    if driver_geojson::write_geo_geojson(&mut json, g).is_err() {
        return Ok(()); // Err legittimo (es. Z/M): niente da verificare
    }
    let geom: geojson::Geometry = serde_json::from_slice(&json).map_err(|e| {
        format!(
            "write_geo_geojson → JSON non valido: {e} | {}",
            String::from_utf8_lossy(&json)
        )
    })?;
    let mut wkb = Vec::new();
    driver_geojson::wkb_from_gj_value(&geom.value, &mut wkb)
        .map_err(|e| format!("wkb_from_gj_value fallisce sul nostro output: {e}"))?;
    let g2 = from_wkb(&wkb, &LIM)
        .map_err(|e| format!("WKB da wkb_from_gj_value non decodificabile: {e}"))?;
    // I conteggi delle coordinate devono combaciare (stessa geometria).
    let (mut a, mut b) = (Vec::new(), Vec::new());
    collect_coords(g, &mut a);
    collect_coords(&g2, &mut b);
    if a.len() != b.len() {
        return Err(format!(
            "round-trip geojson perde coordinate: {} → {}",
            a.len(),
            b.len()
        ));
    }
    Ok(())
}

/// wkb_from_gj_value su un `geojson::Value` sintetico: non deve panic, e se
/// produce WKB dev'essere decodificabile.
fn check_gj_value(v: &geojson::Value) -> Result<(), String> {
    let mut wkb = Vec::new();
    // Se produce WKB (Ok), dev'essere decodificabile; Err = rifiuto pulito.
    if driver_geojson::wkb_from_gj_value(v, &mut wkb).is_ok() {
        from_wkb(&wkb, &LIM)
            .map_err(|e| format!("wkb_from_gj_value produce WKB indecodificabile: {e}"))?;
    }
    Ok(())
}

/// L'adattatore WKT dimensionale di CSV/XLSX non deve mai panic; se accetta,
/// deve produrre WKT e WKB rileggibili senza cambiare tipo o dimensioni.
fn check_wkt(s: &str) -> Result<(), String> {
    if let Ok(geometry) = parse_wkt(s) {
        let text = format_wkt(&geometry)
            .map_err(|e| format!("serializzazione WKT dopo parse OK fallisce: {e}"))?;
        let reparsed =
            parse_wkt(&text).map_err(|e| format!("WKT prodotto non rileggibile: {e}"))?;
        if geometry != reparsed {
            return Err("round-trip WKT altera geometria o ordinate".to_owned());
        }
        let encoded = encode_wkb(&geometry, WkbFlavor::Iso)
            .map_err(|e| format!("WKB dopo WKT OK fallisce: {e}"))?;
        let decoded = decode_wkb(&encoded, &LIM)
            .map_err(|e| format!("WKB da WKT non ri-decodificabile: {e}"))?;
        if geometry != decoded {
            return Err("round-trip WKT→WKB altera geometria o ordinate".to_owned());
        }
    }
    Ok(())
}

/// geojson::Value casuale, profondità limitata (≤4, come il limite serde reale).
fn rand_gj_value(rng: &mut Rng, depth: usize) -> geojson::Value {
    use geojson::Value::*;
    let pos = |rng: &mut Rng| -> Vec<f64> { (0..rng.below(4)).map(|_| rng.f64()).collect() };
    let ring = |rng: &mut Rng| -> Vec<Vec<f64>> { (0..rng.below(6)).map(|_| pos(rng)).collect() };
    let poly =
        |rng: &mut Rng| -> Vec<Vec<Vec<f64>>> { (0..rng.below(3)).map(|_| ring(rng)).collect() };
    let n = if depth >= 4 {
        rng.below(6)
    } else {
        rng.below(7)
    };
    match n {
        0 => Point(pos(rng)),
        1 => MultiPoint((0..rng.below(5)).map(|_| pos(rng)).collect()),
        2 => LineString(ring(rng)),
        3 => MultiLineString((0..rng.below(3)).map(|_| ring(rng)).collect()),
        4 => Polygon(poly(rng)),
        5 => MultiPolygon((0..rng.below(3)).map(|_| poly(rng)).collect()),
        _ => GeometryCollection(
            (0..rng.below(3))
                .map(|_| geojson::Geometry::new(rand_gj_value(rng, depth + 1)))
                .collect(),
        ),
    }
}

// --- artefatti -------------------------------------------------------------

fn fnv1a(b: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn save(dir: &Path, target: &str, input: &[u8], why: &str) {
    let h = fnv1a(input) ^ fnv1a(why.as_bytes());
    let base = dir.join(format!("{target}-{h:016x}"));
    let _ = std::fs::write(base.with_extension("bin"), input);
    let _ = std::fs::write(
        base.with_extension("txt"),
        format!(
            "target: {target}\nviolazione: {why}\nlen: {}\n",
            input.len()
        ),
    );
}

fn panic_msg(e: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = e.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic (payload non testuale)".to_owned()
    }
}

// --- replay (triage di un artefatto) ---------------------------------------

fn replay(path: &str) {
    let data = std::fs::read(path).expect("lettura artefatto");
    println!("replay {path} ({} byte)", data.len());
    // WKB
    let r = catch_unwind(AssertUnwindSafe(|| check_wkb(&data)));
    println!("  check_wkb → {r:?}");
    // geojson::Value
    if let Ok(geom) = serde_json::from_slice::<geojson::Geometry>(&data) {
        let r = catch_unwind(AssertUnwindSafe(|| check_gj_value(&geom.value)));
        println!("  check_gj_value(parsed) → {r:?}");
    }
    if let Ok(text) = std::str::from_utf8(&data) {
        let r = catch_unwind(AssertUnwindSafe(|| check_wkt(text)));
        println!("  check_wkt(utf8) → {r:?}");
    }
    println!("  from_wkb → {:?}", from_wkb(&data, &LIM).map(|_| "Ok"));
    println!("  wkb_bbox → {:?}", driver_geoparquet::wkb_bbox(&data));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--export-corpus") {
        let root = args
            .get(2)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/work/fuzz/corpus"));
        export_corpus(&root);
        return;
    }
    if args.get(1).map(String::as_str) == Some("--replay-shared-corpus") {
        let root = match args.get(2) {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from("/work/fuzz/shared-corpus"),
        };
        match replay_shared_corpus(&root) {
            Ok(report) => {
                let encoded = match serde_json::to_string_pretty(&report) {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        eprintln!("serializzazione report corpus fallita: {error}");
                        std::process::exit(1);
                    }
                };
                if let Some(output) = args.get(3).map(PathBuf::from) {
                    if let Err(error) = std::fs::write(&output, format!("{encoded}\n")) {
                        eprintln!("scrittura {} fallita: {error}", output.display());
                        std::process::exit(1);
                    }
                } else {
                    println!("{encoded}");
                }
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(path) = args.get(1) {
        replay(path);
        return;
    }

    let secs: u64 = std::env::var("PLENORA_FUZZ_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let seed: u64 = std::env::var("PLENORA_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E3779B97F4A7C15)
                | 1
        });
    let out = PathBuf::from(
        std::env::var("PLENORA_FUZZ_OUT").unwrap_or_else(|_| "/work/fuzz-findings".to_owned()),
    );
    std::fs::create_dir_all(&out).ok();

    // Silenzia l'hook di panic di default: le catturiamo noi.
    std::panic::set_hook(Box::new(|_| {}));

    let mut rng = Rng(seed);
    let wkb = wkb_seeds();
    let gj = geojson_seeds();
    let fc = fc_seeds();
    let wkt = wkt_seeds();
    let deadline = Duration::from_secs(secs);
    let start = Instant::now();
    let mut seen: HashSet<u64> = HashSet::new();
    let mut iters: u64 = 0;
    let mut findings: u64 = 0;
    let mut last_report = Instant::now();

    println!(
        "plenora-fuzz: seed={seed} durata={secs}s out={}",
        out.display()
    );

    while start.elapsed() < deadline {
        for _ in 0..20_000 {
            iters += 1;
            // Distribuzione sbilanciata verso le superfici meno sature (il
            // codec WKB ha già retto 13G iterazioni pulite su 2 chunk).
            match rng.below(10) {
                // ~30%: WKB (mutato o casuale)
                0..=2 => {
                    let input = if rng.below(5) == 0 {
                        let n = rng.below(80);
                        (0..n).map(|_| rng.byte()).collect::<Vec<u8>>()
                    } else {
                        let idx = rng.below(wkb.len());
                        mutate(&mut rng, &wkb[idx])
                    };
                    if let Err(why) = run(|| check_wkb(&input)) {
                        report(&out, "wkb", &input, &why, &mut seen, &mut findings);
                    }
                }
                // ~20%: FeatureCollection TEXT → deserializer completo (pass-1+2
                // sincrono: cattura panic e disallineamenti colonne).
                3..=4 => {
                    let idx = rng.below(fc.len());
                    let input = mutate(&mut rng, &fc[idx]);
                    // Err = JSON invalido (rifiuto legittimo). Solo un PANIC è bug.
                    if let Err(why) = run(|| {
                        let _ = driver_geojson::__fuzz_read_geojson(&input);
                        Ok(())
                    }) {
                        report(&out, "fc", &input, &why, &mut seen, &mut findings);
                    }
                }
                // ~20%: WKT (mutato) — percorso geometria csv/xls
                5..=6 => {
                    let idx = rng.below(wkt.len());
                    let input = mutate(&mut rng, &wkt[idx]);
                    let s = String::from_utf8_lossy(&input);
                    if let Err(why) = run(|| check_wkt(s.as_ref())) {
                        report(&out, "wkt", &input, &why, &mut seen, &mut findings);
                    }
                }
                // ~20%: geojson geometria TEXT (mutato)
                7..=8 => {
                    let idx = rng.below(gj.len());
                    let input = mutate(&mut rng, &gj[idx]);
                    if let Err(why) = run(|| {
                        if let Ok(geom) = serde_json::from_slice::<geojson::Geometry>(&input) {
                            check_gj_value(&geom.value)?;
                        }
                        Ok(())
                    }) {
                        report(&out, "gjtext", &input, &why, &mut seen, &mut findings);
                    }
                }
                // ~10%: geojson::Value sintetico
                _ => {
                    let v = rand_gj_value(&mut rng, 0);
                    let bytes =
                        serde_json::to_vec(&geojson::Geometry::new(v.clone())).unwrap_or_default();
                    if let Err(why) = run(|| check_gj_value(&v)) {
                        report(&out, "gjval", &bytes, &why, &mut seen, &mut findings);
                    }
                }
            }
        }
        if last_report.elapsed() >= Duration::from_secs(30) {
            let el = start.elapsed().as_secs_f64().max(0.001);
            println!(
                "  {:.0}s  iter={}  {:.0}k/s  findings={}",
                el,
                iters,
                iters as f64 / el / 1000.0,
                findings
            );
            let _ = std::io::stdout().flush();
            last_report = Instant::now();
        }
    }
    println!(
        "FINE: iter={iters} findings={findings} durata={:.0}s",
        start.elapsed().as_secs_f64()
    );
}

/// Esegue un check catturando eventuali panic come violazione.
fn run(f: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(e) => Err(format!("PANIC: {}", panic_msg(e))),
    }
}

fn report(
    out: &Path,
    target: &str,
    input: &[u8],
    why: &str,
    seen: &mut HashSet<u64>,
    findings: &mut u64,
) {
    let key = fnv1a(why.as_bytes());
    if seen.insert(key) {
        *findings += 1;
        save(out, target, input, why);
        eprintln!("[FINDING {target}] {why}");
    }
}

#[cfg(test)]
mod shared_corpus_tests {
    use super::*;

    #[test]
    fn checked_in_shared_corpus_matches_io_codec() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/shared-corpus");
        if let Err(error) = replay_shared_corpus(&root) {
            panic!("replay corpus condiviso fallito: {error}");
        }
    }
}
