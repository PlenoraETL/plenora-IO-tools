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
//! Env: PLENORA_FUZZ_SECONDS, PLENORA_FUZZ_SEED, PLENORA_FUZZ_OUT.

use std::collections::HashSet;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use geo_types::Geometry;
use plenora_core::limits::WkbLimits;
use plenora_core::wkb::{from_wkb, to_wkb};
use wkt::TryFromWkt;

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
        Geometry::MultiLineString(MultiLineString(vec![LineString(vec![c(0., 0.), c(9., 9.)])])),
        Geometry::MultiPolygon(MultiPolygon(vec![Polygon::new(ext, vec![])])),
        Geometry::GeometryCollection(GeometryCollection(vec![
            Geometry::Point(Point::new(7., 8.)),
            Geometry::Polygon(Polygon::new(
                LineString(vec![c(0., 0.), c(1., 0.), c(1., 1.), c(0., 0.)]),
                vec![],
            )),
        ])),
    ];
    gs.iter().map(|g| to_wkb(g).unwrap()).collect()
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

/// WKT valido dei 7 tipi: percorso geometria di csv/xls (`try_from_wkt_str`,
/// crate `wkt` di terze parti — il parser più esposto a panic su input ostile).
fn wkt_seeds() -> Vec<Vec<u8>> {
    [
        "POINT (1 2)",
        "LINESTRING (0 0, 1 1, 2 2)",
        "POLYGON ((0 0, 4 0, 4 4, 0 0), (1 1, 2 1, 2 2, 1 1))",
        "MULTIPOINT ((0 0), (5 5))",
        "MULTILINESTRING ((0 0, 1 1), (2 2, 3 3))",
        "MULTIPOLYGON (((0 0, 1 0, 1 1, 0 0)))",
        "GEOMETRYCOLLECTION (POINT (7 8), LINESTRING (0 0, 1 1))",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
}

/// FeatureCollection eterogenee: proprietà disomogenee, tipi misti, geometria
/// null e properties null — per stressare pass-1 + pass-2 (allineamento colonne).
fn fc_seeds() -> Vec<Vec<u8>> {
    [
        r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Point","coordinates":[1,2]},"properties":{"a":1,"b":"x","c":true}}]}"#,
        r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":null,"properties":{"a":2}},{"type":"Feature","geometry":{"type":"Point","coordinates":[3,3]},"properties":null},{"type":"Feature","geometry":{"type":"LineString","coordinates":[[0,0],[1,1]]},"properties":{"b":"y","d":9.5}}]}"#,
        r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]},"properties":{"n":-3,"arr":[1,2,3],"obj":{"k":1}}}]}"#,
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
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

// --- i check (ognuno ritorna Err(descrizione) su violazione) ---------------

const LIM: WkbLimits = WkbLimits {
    max_cell_bytes: 1 << 20,
    max_components: 1 << 20,
    max_depth: 64,
};

/// Invarianti sul codec WKB e sullo scanner bbox, da byte grezzi.
fn check_wkb(data: &[u8]) -> Result<(), String> {
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
        let finite: Vec<_> = cs.iter().filter(|(x, y)| x.is_finite() && y.is_finite()).collect();
        if !finite.is_empty() {
            let minx = finite.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min);
            let miny = finite.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
            let maxx = finite.iter().map(|(x, _)| *x).fold(f64::NEG_INFINITY, f64::max);
            let maxy = finite.iter().map(|(_, y)| *y).fold(f64::NEG_INFINITY, f64::max);
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

/// `try_from_wkt_str` (percorso csv/xls) non deve MAI panic; se accetta, la
/// geometria dev'essere codificabile in WKB e ri-decodificabile.
fn check_wkt(s: &str) -> Result<(), String> {
    if let Ok(g) = geo_types::Geometry::<f64>::try_from_wkt_str(s) {
        let enc = to_wkb(&g).map_err(|e| format!("to_wkb dopo WKT OK fallisce: {e}"))?;
        from_wkb(&enc, &LIM).map_err(|e| format!("WKB da WKT non ri-decodificabile: {e}"))?;
    }
    Ok(())
}

/// geojson::Value casuale, profondità limitata (≤4, come il limite serde reale).
fn rand_gj_value(rng: &mut Rng, depth: usize) -> geojson::Value {
    use geojson::Value::*;
    let pos = |rng: &mut Rng| -> Vec<f64> { (0..rng.below(4)).map(|_| rng.f64()).collect() };
    let ring = |rng: &mut Rng| -> Vec<Vec<f64>> { (0..rng.below(6)).map(|_| pos(rng)).collect() };
    let poly = |rng: &mut Rng| -> Vec<Vec<Vec<f64>>> { (0..rng.below(3)).map(|_| ring(rng)).collect() };
    let n = if depth >= 4 { rng.below(6) } else { rng.below(7) };
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
        format!("target: {target}\nviolazione: {why}\nlen: {}\n", input.len()),
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
    println!("  from_wkb → {:?}", from_wkb(&data, &LIM).map(|_| "Ok"));
    println!("  wkb_bbox → {:?}", driver_geoparquet::wkb_bbox(&data));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
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
    let out = PathBuf::from(std::env::var("PLENORA_FUZZ_OUT").unwrap_or_else(|_| "/work/fuzz-findings".to_owned()));
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

    println!("plenora-fuzz: seed={seed} durata={secs}s out={}", out.display());

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
                    let bytes = serde_json::to_vec(&geojson::Geometry::new(v.clone()))
                        .unwrap_or_default();
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
