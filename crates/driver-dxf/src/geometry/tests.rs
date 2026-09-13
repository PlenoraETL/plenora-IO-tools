//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// Confronto esatto voluto: identità e composizioni con fattori binari
// esatti devono restituire i valori bit a bit, non "vicini".
#[allow(clippy::float_cmp)]
#[test]
fn identity_and_composition_behave() {
    let point = [3.0, 4.0];
    assert_eq!(Transform::IDENTITY.apply(point), point);

    // Rotazione 90° attorno all'origine: (1,0) -> (0,1).
    let rot = Transform::insert([0.0, 0.0], 90.0, 1.0, 1.0);
    let rotated = rot.apply([1.0, 0.0]);
    assert!(close(rotated[0], 0.0) && close(rotated[1], 1.0));

    // Composizione: prima scala x2, poi trasla (10,0).
    let scale = Transform::insert([0.0, 0.0], 0.0, 2.0, 2.0);
    let translate = Transform::insert([10.0, 0.0], 0.0, 1.0, 1.0);
    let composed = translate.then(scale);
    assert_eq!(composed.apply([1.0, 1.0]), [12.0, 2.0]);
}

// Confronto esatto voluto: l'OCS identità e quello degenere non devono
// perturbare di un solo bit le coordinate.
#[allow(clippy::float_cmp)]
#[test]
fn ocs_mirrors_x_for_reversed_extrusion() {
    // Extrusion (0,0,1): identità.
    let identity = Transform::ocs([0.0, 0.0, 1.0]);
    assert_eq!(identity.apply([3.0, 4.0]), [3.0, 4.0]);
    // Extrusion (0,0,-1): la X viene specchiata, la Y resta.
    let mirror = Transform::ocs([0.0, 0.0, -1.0]);
    let m = mirror.apply([3.0, 4.0]);
    assert!(close(m[0], -3.0) && close(m[1], 4.0));
    // Extrusion degenere (0,0,0): trattata come identità, niente panico.
    assert_eq!(
        Transform::ocs([0.0, 0.0, 0.0]).apply([1.0, 2.0]),
        [1.0, 2.0]
    );
}

#[test]
fn insert_composes_translation_rotation_scale() {
    // Scala 2, ruota 90°, trasla (5,5): il punto locale (1,0) ->
    // scala (2,0) -> ruota (0,2) -> trasla (5,7).
    let transform = Transform::insert([5.0, 5.0], 90.0, 2.0, 2.0);
    let mapped = transform.apply([1.0, 0.0]);
    assert!(close(mapped[0], 5.0) && close(mapped[1], 7.0));
}

// Confronto esatto voluto: l'OCS identità e quello a normale invertita
// devono restituire le coordinate bit a bit (segno incluso).
#[allow(clippy::float_cmp)]
#[test]
fn transform3_preserves_z_through_insert_and_ocs() {
    let insert = Transform3::insert([5.0, 6.0, 7.0], 90.0, 2.0, 3.0, 4.0);
    let mapped = insert.apply([1.0, 0.0, 2.0]);
    assert!(close(mapped[0], 5.0));
    assert!(close(mapped[1], 8.0));
    assert!(close(mapped[2], 15.0));

    let identity = Transform3::ocs([0.0, 0.0, 1.0]);
    assert_eq!(identity.apply([1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
    let reversed = Transform3::ocs([0.0, 0.0, -1.0]);
    assert_eq!(reversed.apply([1.0, 2.0, 3.0]), [-1.0, 2.0, -3.0]);
}

// Niente `hypot`: l'asserzione replica la stessa forma sqrt(x²+y²) usata
// dal codice tassellato, che non puo' cambiare arrotondamento.
#[allow(clippy::imprecise_flops)]
#[test]
fn semicircle_bulge_follows_the_ccw_convention() {
    // Bulge 1 da (0,0) a (2,0): semicerchio di centro (1,0) raggio 1.
    let center = bulge_center([0.0, 0.0], [2.0, 0.0], 1.0);
    assert!(close(center[0], 1.0) && close(center[1], 0.0));
    // Convenzione DXF: bulge positivo = arco ANTIORARIO dal primo al
    // secondo vertice, che qui passa dal basso e tocca (1,-1).
    let ccw = tessellate_bulge([0.0, 0.0], [2.0, 0.0], 1.0, 24);
    assert!(ccw.iter().any(|p| close(p[0], 1.0) && close(p[1], -1.0)));
    // Bulge negativo: arco orario, colmo in alto (1,1).
    let cw = tessellate_bulge([0.0, 0.0], [2.0, 0.0], -1.0, 24);
    assert!(cw.iter().any(|p| close(p[0], 1.0) && close(p[1], 1.0)));
    // Tutti i punti sul cerchio unitario centrato in (1,0).
    for point in ccw.iter().chain(cw.iter()) {
        let r = ((point[0] - 1.0).powi(2) + point[1].powi(2)).sqrt();
        assert!(close(r, 1.0));
    }
}

#[test]
fn un_bulge_fra_coordinate_enormi_non_perde_l_arco() {
    // Il punto medio non e' `0.5 * (a + b)`, ed e' una differenza di
    // **comportamento**, non di forma.
    //
    // Il lettore DXF valida le coordinate soltanto per finitezza: `1e308`
    // passa. Due ascisse la cui **somma** supera `f64::MAX` facevano
    // traboccare quella somma a infinito prima che il fattore `0.5` la
    // riportasse in scala, e `tessellate_bulge` scartava l'arco sul proprio
    // controllo `center.is_finite()`. Il file era valido, il difetto nostro,
    // e l'arco spariva in silenzio.
    //
    // `f64::midpoint` calcola la media senza passare dalla somma, quindi il
    // centro resta finito e l'arco viene tassellato.
    //
    // Le due ascisse sono **dello stesso segno** e distanti fra loro: e' la
    // somma a dover traboccare, non la differenza. La prima stesura di
    // questa sonda usava `-lontano` e `+lontano`, dove a traboccare era
    // `p2[0] - p1[0]` nel termine dell'offset -- un altro punto del calcolo,
    // che `midpoint` non tocca. La sonda e' diventata rossa e aveva ragione.
    let alta = f64::MAX * 0.6;
    let bassa = f64::MAX * 0.5;
    assert!(!(alta + bassa).is_finite(), "la somma deve traboccare");
    assert!((alta - bassa).is_finite(), "la differenza no");

    let centro = bulge_center([alta, 0.0], [bassa, 0.0], 1.0);
    assert!(
        centro[0].is_finite() && centro[1].is_finite(),
        "il centro deve restare finito: {centro:?}"
    );
    assert!(
        close(centro[0], f64::MAX * 0.55),
        "e valere la media esatta: {}",
        centro[0]
    );

    // E l'arco esce davvero, invece di sparire: e' cio' che il prodotto
    // consegna, e il controllo `is_finite` di `tessellate_bulge` lo
    // scartava.
    let punti = tessellate_bulge([alta, 0.0], [bassa, 0.0], 1.0, 24);
    assert!(
        !punti.is_empty(),
        "l'arco fra coordinate enormi non deve piu' essere scartato"
    );
}

#[test]
fn quarter_bulge_center_is_exact() {
    // 90°: bulge = tan(π/8). Da (1,0) a (0,1) attorno all'origine.
    let bulge = (std::f64::consts::PI / 8.0).tan();
    let center = bulge_center([1.0, 0.0], [0.0, 1.0], bulge);
    assert!(close(center[0], 0.0) && close(center[1], 0.0));
}

#[test]
fn degenerate_curves_yield_no_points() {
    assert!(tessellate_bulge([0.0, 0.0], [1.0, 0.0], 0.0, 24).is_empty());
    assert!(tessellate_arc([0.0, 0.0], 0.0, 0.0, 90.0, 24).is_empty());
    assert!(tessellate_circle([0.0, 0.0], -1.0, 24).is_empty());
}

// Niente mul_add/FMA: l'asserzione replica la stessa forma dell'equazione
// dell'ellisse, che non puo' cambiare arrotondamento.
#[allow(clippy::suboptimal_flops)]
#[test]
fn ellipse_full_is_closed_and_respects_axes() {
    // Ellisse assi-allineata: semiasse maggiore 4 (lungo X), ratio 0.5.
    let ring = tessellate_ellipse([0.0, 0.0], [4.0, 0.0], 0.5, 0.0, TAU, 24);
    assert_eq!(ring.first(), ring.last());
    // Estremi: (±4, 0) e (0, ±2).
    assert!(ring.iter().any(|p| close(p[0], 4.0) && close(p[1], 0.0)));
    assert!(ring.iter().any(|p| close(p[0], 0.0) && close(p[1], 2.0)));
    // Ogni punto soddisfa (x/4)^2 + (y/2)^2 = 1.
    for p in &ring {
        assert!(close((p[0] / 4.0).powi(2) + (p[1] / 2.0).powi(2), 1.0));
    }
}

#[test]
fn ellipse3_preserves_tilted_plane_z() {
    let ring = tessellate_ellipse3(
        [1.0, 2.0, 3.0],
        [2.0, 0.0, 2.0],
        [-1.0, 0.0, 1.0],
        0.5,
        0.0,
        TAU,
        24,
    );
    assert_eq!(ring.first(), ring.last());
    assert!(ring.iter().any(|point| (point[2] - 5.0).abs() < 1e-9));
    assert!(ring.iter().any(|point| (point[2] - 1.0).abs() < 1e-9));
}

#[test]
fn spline_with_collinear_controls_is_a_line() {
    // Spline di grado 1 (poligonale) su punti collineari: resta sulla retta.
    let controls = vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]];
    let knots = vec![0.0, 0.0, 1.0, 2.0, 2.0]; // n=3, p=1 -> n+p+1=5
    let curve = tessellate_spline(1, &knots, &controls, &[], 9);
    assert!(curve.len() >= 2);
    for p in &curve {
        assert!(close(p[0], p[1]), "punto {p:?} fuori dalla retta y=x");
    }
    assert!(close(curve.first().unwrap()[0], 0.0));
    assert!(close(curve.last().unwrap()[0], 2.0));
}

#[test]
fn spline_curve_stays_in_control_hull_and_falls_back_when_malformed() {
    // Quadratica: la curva deve restare nel bounding box dei controlli.
    let controls = vec![[0.0, 0.0], [1.0, 2.0], [2.0, 0.0]];
    let knots = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]; // n=3, p=2 -> 6 knot
    let curve = tessellate_spline(2, &knots, &controls, &[], 12);
    assert!(curve.len() >= 2);
    for p in &curve {
        assert!(p[0] >= -1e-9 && p[0] <= 2.0 + 1e-9);
        assert!(p[1] >= -1e-9 && p[1] <= 2.0 + 1e-9);
    }
    // Estremi interpolati (knot clampato).
    assert!(close(curve.first().unwrap()[0], 0.0) && close(curve.first().unwrap()[1], 0.0));
    assert!(close(curve.last().unwrap()[0], 2.0) && close(curve.last().unwrap()[1], 0.0));
    // Knot vector incoerente -> poligono di controllo.
    let fallback = tessellate_spline(2, &[0.0, 1.0], &controls, &[], 12);
    assert_eq!(fallback, controls);
}

// Niente `hypot`: l'asserzione replica la stessa forma sqrt(x²+y²) usata
// dal codice tassellato, che non puo' cambiare arrotondamento.
#[allow(clippy::imprecise_flops)]
#[test]
fn circle_ring_is_closed_and_on_radius() {
    let ring = tessellate_circle([2.0, 3.0], 5.0, 16);
    assert_eq!(ring.first(), ring.last());
    assert_eq!(ring.len(), 17);
    for point in &ring {
        let r = ((point[0] - 2.0).powi(2) + (point[1] - 3.0).powi(2)).sqrt();
        assert!(close(r, 5.0));
    }
}

#[test]
fn arc_segment_count_scales_with_sweep() {
    // Un arco di 90° usa circa un quarto dei segmenti di un giro intero.
    let quarter = tessellate_arc([0.0, 0.0], 1.0, 0.0, 90.0, 24);
    assert_eq!(quarter.len(), 7); // 6 segmenti + 1
                                  // Un arco che scavalca lo zero (350°->10°) spazza 20°.
    let small = tessellate_arc([0.0, 0.0], 1.0, 350.0, 10.0, 36);
    assert_eq!(small.len(), 3); // 2 segmenti + 1
}
