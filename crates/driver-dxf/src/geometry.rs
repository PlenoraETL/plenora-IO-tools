//! Matematica 2D: trasformazioni affini per l'esplosione dei blocchi INSERT
//! e tassellazione delle primitive curve (bulge, archi, cerchi) in spezzate.
//!
//! Strategia: si tassella sempre in coordinate locali del blocco, poi ogni
//! punto campionato viene mappato attraverso lo stack di trasformazioni. Così
//! una scala non uniforme trasforma un cerchio in un'ellisse correttamente,
//! senza matematica analitica sulle coniche.

use std::f64::consts::TAU;

pub type Point = [f64; 2];
pub type Point3 = [f64; 3];

/// Trasformazione affine 3D usata dal walker DXF. La matrice è memorizzata
/// per righe e applicata a coordinate omogenee `[x, y, z, 1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform3 {
    matrix: [[f64; 4]; 4],
}

impl Transform3 {
    pub const IDENTITY: Self = Self {
        matrix: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    /// Traslazione, rotazione attorno all'asse Z e scala XYZ di un INSERT.
    pub fn insert(
        location: Point3,
        rotation_degrees: f64,
        scale_x: f64,
        scale_y: f64,
        scale_z: f64,
    ) -> Self {
        let theta = rotation_degrees.to_radians();
        let (sin, cos) = theta.sin_cos();
        Self {
            matrix: [
                [cos * scale_x, -sin * scale_y, 0.0, location[0]],
                [sin * scale_x, cos * scale_y, 0.0, location[1]],
                [0.0, 0.0, scale_z, location[2]],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Composizione `self ∘ other`: applica prima `other`, poi `self`.
    pub fn then(self, other: Self) -> Self {
        let mut matrix = [[0.0; 4]; 4];
        for (row, values) in matrix.iter_mut().enumerate() {
            for (column, value) in values.iter_mut().enumerate() {
                *value = (0..4)
                    .map(|index| self.matrix[row][index] * other.matrix[index][column])
                    .sum();
            }
        }
        Self { matrix }
    }

    // Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e romperebbe
    // il determinismo bit-esatto delle coordinate trasformate.
    #[allow(clippy::suboptimal_flops)]
    pub fn apply(&self, point: Point3) -> Point3 {
        [
            self.matrix[0][0] * point[0]
                + self.matrix[0][1] * point[1]
                + self.matrix[0][2] * point[2]
                + self.matrix[0][3],
            self.matrix[1][0] * point[0]
                + self.matrix[1][1] * point[1]
                + self.matrix[1][2] * point[2]
                + self.matrix[1][3],
            self.matrix[2][0] * point[0]
                + self.matrix[2][1] * point[1]
                + self.matrix[2][2] * point[2]
                + self.matrix[2][3],
        ]
    }

    /// Arbitrary-axis algorithm DXF completo: le colonne della matrice sono
    /// gli assi X/Y dell'OCS e la normale Z, tutti espressi in WCS.
    pub fn ocs(normal: Point3) -> Self {
        let n = normalize3(normal).unwrap_or([0.0, 0.0, 1.0]);
        let arbitrary = if n[0].abs() < 1.0 / 64.0 && n[1].abs() < 1.0 / 64.0 {
            cross3([0.0, 1.0, 0.0], n)
        } else {
            cross3([0.0, 0.0, 1.0], n)
        };
        let ax = normalize3(arbitrary).unwrap_or([1.0, 0.0, 0.0]);
        let ay = normalize3(cross3(n, ax)).unwrap_or([0.0, 1.0, 0.0]);
        Self {
            matrix: [
                [ax[0], ay[0], n[0], 0.0],
                [ax[1], ay[1], n[1], 0.0],
                [ax[2], ay[2], n[2], 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }
}

/// Trasformazione affine 2D: x' = a·x + c·y + e ; y' = b·x + d·y + f.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

#[cfg(test)]
impl Transform {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// Trasformazione di un INSERT: trasla nell'inserimento, ruota, scala.
    /// Applicata a coordinate locali del blocco: `translate ∘ rotate ∘ scale`.
    pub fn insert(location: Point, rotation_degrees: f64, scale_x: f64, scale_y: f64) -> Self {
        let theta = rotation_degrees.to_radians();
        let (sin, cos) = theta.sin_cos();
        // rotate ∘ scale
        let a = cos * scale_x;
        let b = sin * scale_x;
        let c = -sin * scale_y;
        let d = cos * scale_y;
        Self {
            a,
            b,
            c,
            d,
            e: location[0],
            f: location[1],
        }
    }

    /// Composizione `self ∘ other`: applica prima `other`, poi `self`.
    // Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e romperebbe
    // il determinismo bit-esatto della composizione affine.
    // `suspicious_operation_groupings` e' un falso positivo: gli indici
    // asimmetrici sono il prodotto righe-per-colonne di due matrici 3x3
    // affini, verificato termine a termine.
    #[allow(clippy::suboptimal_flops, clippy::suspicious_operation_groupings)]
    pub fn then(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    // Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e romperebbe
    // il determinismo bit-esatto delle coordinate trasformate.
    #[allow(clippy::suboptimal_flops)]
    pub fn apply(&self, point: Point) -> Point {
        [
            self.a * point[0] + self.c * point[1] + self.e,
            self.b * point[0] + self.d * point[1] + self.f,
        ]
    }

    /// Trasformazione OCS -> WCS per un'entità con la data extrusion direction
    /// (algoritmo dell'asse arbitrario del DXF, proiettato in 2D scartando la
    /// quota). L'extrusion (0,0,1) dà l'identità; (0,0,-1) specchia la X. Senza
    /// questa mappa le entità in coordinate oggetto mirrorate finiscono a X
    /// negativa (bug trovato dall'oracolo GDAL sul corpus RFI).
    pub fn ocs(normal: [f64; 3]) -> Self {
        let n = normalize3(normal).unwrap_or([0.0, 0.0, 1.0]);
        let arbitrary = if n[0].abs() < 1.0 / 64.0 && n[1].abs() < 1.0 / 64.0 {
            cross3([0.0, 1.0, 0.0], n)
        } else {
            cross3([0.0, 0.0, 1.0], n)
        };
        let ax = normalize3(arbitrary).unwrap_or([1.0, 0.0, 0.0]);
        let ay = normalize3(cross3(n, ax)).unwrap_or([0.0, 1.0, 0.0]);
        Self {
            a: ax[0],
            b: ax[1],
            c: ay[0],
            d: ay[1],
            e: 0.0,
            f: 0.0,
        }
    }
}

// Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e romperebbe il
// determinismo bit-esatto del prodotto vettoriale.
#[allow(clippy::suboptimal_flops)]
fn cross3(u: [f64; 3], v: [f64; 3]) -> [f64; 3] {
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
}

// Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e romperebbe il
// determinismo bit-esatto della norma.
#[allow(clippy::suboptimal_flops)]
fn normalize3(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length.is_finite() && length > 1e-12 {
        Some([v[0] / length, v[1] / length, v[2] / length])
    } else {
        None
    }
}

// `full_circle_segments` e' il numero di segmenti per giro (ARC_SEGMENTS = 24
// nel driver): la conversione in f64 e' esatta. `fraction` sta in [0, 1], quindi
// il prodotto arrotondato sta in [0, full_circle_segments]: non negativo e senza
// troncamento.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn segments_for_angle(sweep: f64, full_circle_segments: usize) -> usize {
    let fraction = (sweep.abs() / TAU).min(1.0);
    ((full_circle_segments as f64 * fraction).round() as usize).max(1)
}

/// Centro dell'arco codificato da un bulge tra due vertici (forma chiusa).
// Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e romperebbe il
// determinismo bit-esatto del centro dell'arco.
#[allow(clippy::suboptimal_flops)]
fn bulge_center(p1: Point, p2: Point, bulge: f64) -> Point {
    let cot = 0.5 * (1.0 / bulge - bulge);
    [
        0.5 * (p1[0] + p2[0]) - cot * 0.5 * (p2[1] - p1[1]),
        0.5 * (p1[1] + p2[1]) + cot * 0.5 * (p2[0] - p1[0]),
    ]
}

/// Campiona l'arco codificato dal bulge, restituendo i punti INTERMEDI
/// (esclusi p1 e p2, che il chiamante possiede già).
// Niente `hypot` al posto di sqrt(x²+y²) e niente mul_add/FMA: entrambi
// cambiano l'arrotondamento IEEE e romperebbero il determinismo bit-esatto
// dell'arco. `step` e `count` sono conteggi di segmenti (<< 2^53): esatti in f64.
#[allow(
    clippy::imprecise_flops,
    clippy::suboptimal_flops,
    clippy::cast_precision_loss
)]
pub fn tessellate_bulge(
    p1: Point,
    p2: Point,
    bulge: f64,
    full_circle_segments: usize,
) -> Vec<Point> {
    if bulge == 0.0 || !bulge.is_finite() {
        return Vec::new();
    }
    let center = bulge_center(p1, p2, bulge);
    if !center[0].is_finite() || !center[1].is_finite() {
        return Vec::new();
    }
    let radius = ((p1[0] - center[0]).powi(2) + (p1[1] - center[1]).powi(2)).sqrt();
    if radius == 0.0 {
        return Vec::new();
    }
    let start = (p1[1] - center[1]).atan2(p1[0] - center[0]);
    let sweep = 4.0 * bulge.atan();
    let count = segments_for_angle(sweep, full_circle_segments);
    let mut points = Vec::with_capacity(count.saturating_sub(1));
    for step in 1..count {
        let angle = start + sweep * (step as f64) / (count as f64);
        points.push([
            center[0] + radius * angle.cos(),
            center[1] + radius * angle.sin(),
        ]);
    }
    points
}

/// Punti di un arco (start/end in gradi, CCW come da specifica DXF).
// Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e romperebbe il
// determinismo bit-esatto dei vertici campionati. `step` e `count` sono
// conteggi di segmenti (<< 2^53): conversione esatta.
#[allow(clippy::suboptimal_flops, clippy::cast_precision_loss)]
pub fn tessellate_arc(
    center: Point,
    radius: f64,
    start_degrees: f64,
    end_degrees: f64,
    full_circle_segments: usize,
) -> Vec<Point> {
    if radius <= 0.0 || !radius.is_finite() {
        return Vec::new();
    }
    let start = start_degrees.to_radians();
    let mut sweep = (end_degrees - start_degrees).to_radians();
    if sweep <= 0.0 {
        sweep += TAU;
    }
    let count = segments_for_angle(sweep, full_circle_segments);
    let mut points = Vec::with_capacity(count + 1);
    for step in 0..=count {
        let angle = start + sweep * (step as f64) / (count as f64);
        points.push([
            center[0] + radius * angle.cos(),
            center[1] + radius * angle.sin(),
        ]);
    }
    points
}

/// Punti di un'ellisse (o arco di ellisse). `major` è il vettore
/// centro->estremo dell'asse maggiore; `ratio` = semiasse minore / maggiore;
/// i parametri sono angoli (rad) come da specifica DXF. Esatta, non
/// approssimata oltre il campionamento.
// Niente `hypot` e niente mul_add/FMA: cambiano l'arrotondamento IEEE e
// romperebbero il determinismo bit-esatto dell'ellisse. `step` e `count` sono
// conteggi di segmenti (<< 2^53): conversione esatta.
#[allow(
    clippy::too_many_arguments,
    clippy::imprecise_flops,
    clippy::suboptimal_flops,
    clippy::cast_precision_loss
)]
#[cfg(test)]
pub fn tessellate_ellipse(
    center: Point,
    major: Point,
    ratio: f64,
    start_parameter: f64,
    end_parameter: f64,
    full_circle_segments: usize,
) -> Vec<Point> {
    let semi_major = (major[0] * major[0] + major[1] * major[1]).sqrt();
    if !semi_major.is_finite() || semi_major <= 0.0 || !ratio.is_finite() {
        return Vec::new();
    }
    let semi_minor = semi_major * ratio;
    let phi = major[1].atan2(major[0]);
    let (sin_phi, cos_phi) = phi.sin_cos();
    let mut sweep = end_parameter - start_parameter;
    if sweep <= 0.0 {
        sweep += TAU;
    }
    let full = (sweep - TAU).abs() < 1e-9;
    let count = segments_for_angle(sweep, full_circle_segments).max(2);
    let mut points = Vec::with_capacity(count + 1);
    for step in 0..=count {
        let t = start_parameter + sweep * (step as f64) / (count as f64);
        let (sin_t, cos_t) = t.sin_cos();
        let x = semi_major * cos_t;
        let y = semi_minor * sin_t;
        points.push([
            center[0] + x * cos_phi - y * sin_phi,
            center[1] + x * sin_phi + y * cos_phi,
        ]);
    }
    if full {
        // Chiude esattamente l'anello.
        let first = points[0];
        if let Some(last) = points.last_mut() {
            *last = first;
        }
    }
    points
}

/// Variante tridimensionale dell'ellisse DXF. `major` è il vettore
/// centro→estremo dell'asse maggiore in WCS; la direzione minore è ricavata
/// dalla normale del piano, così un'ellisse inclinata conserva la quota.
// Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e romperebbe il
// determinismo bit-esatto dell'ellisse 3D. `step` e `count` sono conteggi di
// segmenti (<< 2^53): conversione esatta.
#[allow(
    clippy::too_many_arguments,
    clippy::suboptimal_flops,
    clippy::cast_precision_loss
)]
pub fn tessellate_ellipse3(
    center: Point3,
    major: Point3,
    normal: Point3,
    ratio: f64,
    start_parameter: f64,
    end_parameter: f64,
    full_circle_segments: usize,
) -> Vec<Point3> {
    let semi_major = (major[0] * major[0] + major[1] * major[1] + major[2] * major[2]).sqrt();
    if !semi_major.is_finite() || semi_major <= 0.0 || !ratio.is_finite() {
        return Vec::new();
    }
    let major_direction = [
        major[0] / semi_major,
        major[1] / semi_major,
        major[2] / semi_major,
    ];
    let normal = normalize3(normal).unwrap_or([0.0, 0.0, 1.0]);
    let Some(minor_direction) = normalize3(cross3(normal, major_direction)) else {
        return Vec::new();
    };
    let semi_minor = semi_major * ratio;
    let mut sweep = end_parameter - start_parameter;
    if sweep <= 0.0 {
        sweep += TAU;
    }
    let full = (sweep - TAU).abs() < 1e-9;
    let count = segments_for_angle(sweep, full_circle_segments).max(2);
    let mut points = Vec::with_capacity(count + 1);
    for step in 0..=count {
        let parameter = start_parameter + sweep * (step as f64) / (count as f64);
        let (sin, cos) = parameter.sin_cos();
        points.push([
            center[0] + major[0] * cos + minor_direction[0] * semi_minor * sin,
            center[1] + major[1] * cos + minor_direction[1] * semi_minor * sin,
            center[2] + major[2] * cos + minor_direction[2] * semi_minor * sin,
        ]);
    }
    if full {
        let first = points[0];
        if let Some(last) = points.last_mut() {
            *last = first;
        }
    }
    points
}

/// Campiona una curva spline NURBS con l'algoritmo di de Boor (razionale via
/// coordinate omogenee). Se il knot vector è incoerente ripiega sul poligono
/// di controllo, che delimita la curva vera: un'approssimazione grezza ma mai
/// una geometria inventata.
// `step` e `count` sono conteggi di campioni, limitati dal numero di control
// point dell'entita' DXF (<< 2^53): conversione esatta.
#[allow(clippy::cast_precision_loss)]
#[cfg(test)]
pub fn tessellate_spline(
    degree: usize,
    knots: &[f64],
    control_points: &[Point],
    weights: &[f64],
    samples: usize,
) -> Vec<Point> {
    let n = control_points.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![control_points[0]];
    }
    let p = degree.clamp(1, n - 1);
    let expected = n + p + 1;
    if knots.len() != expected {
        return control_points.to_vec();
    }
    let u_min = knots[p];
    let u_max = knots[n];
    if u_max <= u_min || !u_max.is_finite() || !u_min.is_finite() {
        return control_points.to_vec();
    }
    // Punti di controllo omogenei (x*w, y*w, w).
    let homogeneous: Vec<[f64; 3]> = (0..n)
        .map(|i| {
            let w = weights
                .get(i)
                .copied()
                .filter(|x| x.is_finite() && *x > 0.0)
                .unwrap_or(1.0);
            [control_points[i][0] * w, control_points[i][1] * w, w]
        })
        .collect();

    let count = samples.max(2);
    let mut out = Vec::with_capacity(count);
    for step in 0..count {
        let u = u_min + (u_max - u_min) * (step as f64) / ((count - 1) as f64);
        let point = de_boor(p, knots, &homogeneous, u);
        if point[2].abs() > 1e-12 {
            out.push([point[0] / point[2], point[1] / point[2]]);
        }
    }
    if out.is_empty() {
        return control_points.to_vec();
    }
    out
}

/// Variante tridimensionale della tassellazione NURBS. Conserva la quota dei
/// control point usando coordinate omogenee `(x*w, y*w, z*w, w)`.
// `step` e `count` sono conteggi di campioni, limitati dal numero di control
// point dell'entita' DXF (<< 2^53): conversione esatta.
#[allow(clippy::cast_precision_loss)]
pub fn tessellate_spline3(
    degree: usize,
    knots: &[f64],
    control_points: &[Point3],
    weights: &[f64],
    samples: usize,
) -> Vec<Point3> {
    let n = control_points.len();
    if n < 2 {
        return control_points.to_vec();
    }
    let p = degree.clamp(1, n - 1);
    if knots.len() != n + p + 1 {
        return control_points.to_vec();
    }
    let u_min = knots[p];
    let u_max = knots[n];
    if u_max <= u_min || !u_max.is_finite() || !u_min.is_finite() {
        return control_points.to_vec();
    }
    let homogeneous: Vec<[f64; 4]> = (0..n)
        .map(|index| {
            let weight = weights
                .get(index)
                .copied()
                .filter(|value| value.is_finite() && *value > 0.0)
                .unwrap_or(1.0);
            [
                control_points[index][0] * weight,
                control_points[index][1] * weight,
                control_points[index][2] * weight,
                weight,
            ]
        })
        .collect();

    let count = samples.max(2);
    let mut output = Vec::with_capacity(count);
    for step in 0..count {
        let parameter = u_min + (u_max - u_min) * (step as f64) / ((count - 1) as f64);
        let point = de_boor4(p, knots, &homogeneous, parameter);
        if point[3].abs() > 1e-12 {
            output.push([
                point[0] / point[3],
                point[1] / point[3],
                point[2] / point[3],
            ]);
        }
    }
    if output.is_empty() {
        control_points.to_vec()
    } else {
        output
    }
}

// Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e romperebbe il
// determinismo bit-esatto dell'interpolazione di de Boor.
#[allow(clippy::suboptimal_flops)]
#[cfg(test)]
fn de_boor(p: usize, knots: &[f64], control: &[[f64; 3]], u: f64) -> [f64; 3] {
    let n = control.len();
    // Indice di span k: knots[k] <= u < knots[k+1], con p <= k <= n-1.
    let mut k = p;
    while k < n - 1 && knots[k + 1] <= u {
        k += 1;
    }
    let mut values: Vec<[f64; 3]> = (0..=p)
        .map(|index| control[(index + k).saturating_sub(p).min(n - 1)])
        .collect();
    for iteration in 1..=p {
        for index in (iteration..=p).rev() {
            let knot_index = index + k - p;
            let denominator = knots[knot_index + p + 1 - iteration] - knots[knot_index];
            let alpha = if denominator.abs() > 1e-12 {
                (u - knots[knot_index]) / denominator
            } else {
                0.0
            };
            let previous = values[index - 1];
            let current = values[index];
            values[index] = [
                (1.0 - alpha) * previous[0] + alpha * current[0],
                (1.0 - alpha) * previous[1] + alpha * current[1],
                (1.0 - alpha) * previous[2] + alpha * current[2],
            ];
        }
    }
    values[p]
}

// Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e romperebbe il
// determinismo bit-esatto dell'interpolazione di de Boor.
#[allow(clippy::suboptimal_flops)]
fn de_boor4(p: usize, knots: &[f64], control: &[[f64; 4]], u: f64) -> [f64; 4] {
    let n = control.len();
    let mut k = p;
    while k < n - 1 && knots[k + 1] <= u {
        k += 1;
    }
    let mut values: Vec<[f64; 4]> = (0..=p)
        .map(|index| control[(index + k).saturating_sub(p).min(n - 1)])
        .collect();
    for iteration in 1..=p {
        for index in (iteration..=p).rev() {
            let knot_index = index + k - p;
            let denominator = knots[knot_index + p + 1 - iteration] - knots[knot_index];
            let alpha = if denominator.abs() > 1e-12 {
                (u - knots[knot_index]) / denominator
            } else {
                0.0
            };
            let previous = values[index - 1];
            for (ordinate, previous_ordinate) in values[index].iter_mut().zip(previous) {
                *ordinate = (1.0 - alpha) * previous_ordinate + alpha * *ordinate;
            }
        }
    }
    values[p]
}

/// Anello chiuso che approssima un cerchio.
// Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e romperebbe il
// determinismo bit-esatto dell'anello. `step` e `count` sono conteggi di
// segmenti (<< 2^53): conversione esatta.
#[allow(clippy::suboptimal_flops, clippy::cast_precision_loss)]
pub fn tessellate_circle(center: Point, radius: f64, full_circle_segments: usize) -> Vec<Point> {
    if radius <= 0.0 || !radius.is_finite() {
        return Vec::new();
    }
    let count = full_circle_segments.max(3);
    let mut points = Vec::with_capacity(count + 1);
    for step in 0..count {
        let angle = TAU * (step as f64) / (count as f64);
        points.push([
            center[0] + radius * angle.cos(),
            center[1] + radius * angle.sin(),
        ]);
    }
    points.push(points[0]);
    points
}

#[cfg(test)]
mod tests {
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
}
