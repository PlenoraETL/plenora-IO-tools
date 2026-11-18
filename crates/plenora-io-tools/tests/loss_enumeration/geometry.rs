//! Canonical geometry observations, retaining exact IEEE coordinate bits.
use plenora_io_model::{
    contract::{CoordinateDimensions, GeometryType},
    wkb::{WkbCoordinate, WkbGeometry, WkbValue},
};

type Coordinate = [Option<u64>; 4];

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Observation {
    pub types: Vec<GeometryType>,
    pub dimensions: Vec<CoordinateDimensions>,
    pub srids: Vec<Option<i32>>,
    pub coordinates: Vec<Vec<Coordinate>>,
    pub orientations: Vec<i8>,
}

fn coordinate(c: &WkbCoordinate) -> Coordinate {
    [
        Some(c.x.to_bits()),
        Some(c.y.to_bits()),
        c.z.map(f64::to_bits),
        c.m.map(f64::to_bits),
    ]
}

fn canonical_ring(ring: &[WkbCoordinate]) -> Vec<Coordinate> {
    let mut points: Vec<_> = ring.iter().map(coordinate).collect();
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    // Starting vertex and traversal direction are independent of exact
    // coordinate content. Orientation is retained in its own observation.
    let mut candidates = Vec::new();
    for _ in 0..2 {
        for start in 0..points.len() {
            candidates.push(
                points[start..]
                    .iter()
                    .chain(&points[..start])
                    .copied()
                    .collect::<Vec<_>>(),
            );
        }
        points.reverse();
    }
    candidates.into_iter().min().unwrap_or_default()
}

fn visit(geometry: &WkbGeometry, out: &mut Observation) {
    out.types.push(geometry.geometry_type());
    out.dimensions.push(geometry.dimensions);
    out.srids.push(geometry.srid);
    match &geometry.value {
        WkbValue::Point(c) => out.coordinates.push(vec![coordinate(c)]),
        WkbValue::LineString(c) | WkbValue::CircularString(c) => {
            out.coordinates.push(c.iter().map(coordinate).collect());
        }
        WkbValue::Polygon(rings) | WkbValue::Triangle(rings) => {
            for ring in rings {
                let area: f64 = ring
                    .windows(2)
                    .map(|c| c[1].x.mul_add(-c[0].y, c[0].x * c[1].y))
                    .sum();
                out.orientations.push(if area > 0.0 {
                    1
                } else if area < 0.0 {
                    -1
                } else {
                    0
                });
                out.coordinates.push(canonical_ring(ring));
            }
        }
        WkbValue::MultiPoint(children)
        | WkbValue::MultiLineString(children)
        | WkbValue::MultiPolygon(children)
        | WkbValue::GeometryCollection(children)
        | WkbValue::CompoundCurve(children)
        | WkbValue::CurvePolygon(children)
        | WkbValue::MultiCurve(children)
        | WkbValue::MultiSurface(children)
        | WkbValue::PolyhedralSurface(children)
        | WkbValue::Tin(children) => {
            for child in children {
                visit(child, out);
            }
        }
    }
}

pub(super) fn observe(geometry: &WkbGeometry) -> Observation {
    let mut result = Observation::default();
    visit(geometry, &mut result);
    result
}

#[test]
fn rotations_reversals_and_sub_ulp_changes_are_distinct_observations() {
    use super::super::{child, coordinate};
    let ring = vec![
        coordinate(0.0, 0.0),
        coordinate(1.0, 0.0),
        coordinate(1.0, 1.0),
        coordinate(0.0, 0.0),
    ];
    let original = observe(&child(WkbValue::Polygon(vec![ring.clone()])));
    let rotated = vec![ring[1], ring[2], ring[0], ring[1]];
    assert_eq!(original, observe(&child(WkbValue::Polygon(vec![rotated]))));
    let mut reversed = ring;
    reversed.reverse();
    let opposite = observe(&child(WkbValue::Polygon(vec![reversed])));
    assert_eq!(original.coordinates, opposite.coordinates);
    assert_ne!(original.orientations, opposite.orientations);
    let a = child(WkbValue::Point(coordinate(1.0, 2.0)));
    let b = child(WkbValue::Point(coordinate(
        f64::from_bits(1.0_f64.to_bits() + 1),
        2.0,
    )));
    assert_ne!(observe(&a).coordinates, observe(&b).coordinates);
}
