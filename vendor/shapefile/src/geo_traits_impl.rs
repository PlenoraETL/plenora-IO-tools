use std::hint::unreachable_unchecked;

use crate::{
    Multipoint, MultipointM, MultipointZ, Point, PointM, PointZ, PolygonRing, Polyline, PolylineM,
    PolylineZ, NO_DATA,
};
use geo_traits::{
    CoordTrait, GeometryTrait, LineStringTrait, MultiLineStringTrait, MultiPointTrait,
    MultiPolygonTrait, PointTrait, PolygonTrait, UnimplementedLineString,
    UnimplementedMultiLineString, UnimplementedMultiPoint, UnimplementedMultiPolygon,
    UnimplementedPoint, UnimplementedPolygon,
};

// Shapefile points can't be null, so we implement both traits on them
impl CoordTrait for Point {
    type T = f64;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xy
    }

    fn nth_or_panic(&self, n: usize) -> Self::T {
        match n {
            0 => self.x(),
            1 => self.y(),
            _ => panic!("invalid dimension index"),
        }
    }

    unsafe fn nth_unchecked(&self, n: usize) -> Self::T {
        match n {
            0 => self.x(),
            1 => self.y(),
            _ => unreachable_unchecked(),
        }
    }

    fn x(&self) -> Self::T {
        self.x
    }

    fn y(&self) -> Self::T {
        self.y
    }
}

impl CoordTrait for &Point {
    type T = f64;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xy
    }

    fn nth_or_panic(&self, n: usize) -> Self::T {
        match n {
            0 => self.x(),
            1 => self.y(),
            _ => panic!("invalid dimension index"),
        }
    }

    unsafe fn nth_unchecked(&self, n: usize) -> Self::T {
        match n {
            0 => self.x(),
            1 => self.y(),
            _ => unreachable_unchecked(),
        }
    }

    fn x(&self) -> Self::T {
        self.x
    }

    fn y(&self) -> Self::T {
        self.y
    }
}

impl PointTrait for Point {
    type CoordType<'a>
        = &'a Point
    where
        Self: 'a;

    fn coord(&self) -> Option<Self::CoordType<'_>> {
        Some(self)
    }
}

impl PointTrait for &Point {
    type CoordType<'a>
        = &'a Point
    where
        Self: 'a;

    fn coord(&self) -> Option<Self::CoordType<'_>> {
        Some(self)
    }
}

// Shapefile points can't be null, so we implement both traits on them
impl CoordTrait for PointM {
    type T = f64;

    fn dim(&self) -> geo_traits::Dimensions {
        if self.m <= NO_DATA {
            geo_traits::Dimensions::Xy
        } else {
            geo_traits::Dimensions::Xym
        }
    }

    fn nth_or_panic(&self, n: usize) -> Self::T {
        match n {
            0 => self.x(),
            1 => self.y(),
            2 => self.m,
            _ => panic!("invalid dimension index"),
        }
    }

    fn x(&self) -> Self::T {
        self.x
    }

    fn y(&self) -> Self::T {
        self.y
    }
}

impl CoordTrait for &PointM {
    type T = f64;

    fn dim(&self) -> geo_traits::Dimensions {
        if self.m <= NO_DATA {
            geo_traits::Dimensions::Xy
        } else {
            geo_traits::Dimensions::Xym
        }
    }

    fn nth_or_panic(&self, n: usize) -> Self::T {
        match n {
            0 => self.x(),
            1 => self.y(),
            2 => self.m,
            _ => panic!("invalid dimension index"),
        }
    }

    fn x(&self) -> Self::T {
        self.x
    }

    fn y(&self) -> Self::T {
        self.y
    }
}

impl PointTrait for PointM {
    type CoordType<'a>
        = &'a PointM
    where
        Self: 'a;

    fn coord(&self) -> Option<Self::CoordType<'_>> {
        Some(self)
    }
}

impl PointTrait for &PointM {
    type CoordType<'a>
        = &'a PointM
    where
        Self: 'a;

    fn coord(&self) -> Option<Self::CoordType<'_>> {
        Some(self)
    }
}

// Shapefile points can't be null, so we implement both traits on them
impl CoordTrait for PointZ {
    type T = f64;

    fn dim(&self) -> geo_traits::Dimensions {
        if self.m <= NO_DATA {
            geo_traits::Dimensions::Xyz
        } else {
            geo_traits::Dimensions::Xyzm
        }
    }

    fn nth_or_panic(&self, n: usize) -> Self::T {
        match n {
            0 => self.x(),
            1 => self.y(),
            2 => self.z,
            3 => {
                if self.m > NO_DATA {
                    self.m
                } else {
                    panic!("asked for 4th item from coordinate but this coordinate does not have 4 dimensions.")
                }
            }
            _ => panic!("invalid dimension index"),
        }
    }

    fn x(&self) -> Self::T {
        self.x
    }

    fn y(&self) -> Self::T {
        self.y
    }
}

impl CoordTrait for &PointZ {
    type T = f64;

    fn dim(&self) -> geo_traits::Dimensions {
        if self.m <= NO_DATA {
            geo_traits::Dimensions::Xyz
        } else {
            geo_traits::Dimensions::Xyzm
        }
    }

    fn nth_or_panic(&self, n: usize) -> Self::T {
        match n {
            0 => self.x(),
            1 => self.y(),
            2 => self.z,
            3 => {
                if self.m > NO_DATA {
                    self.m
                } else {
                    panic!("asked for 4th item from coordinate but this coordinate does not have 4 dimensions.")
                }
            }
            _ => panic!("invalid dimension index"),
        }
    }

    fn x(&self) -> Self::T {
        self.x
    }

    fn y(&self) -> Self::T {
        self.y
    }
}

impl PointTrait for PointZ {
    type CoordType<'a>
        = &'a PointZ
    where
        Self: 'a;

    fn coord(&self) -> Option<Self::CoordType<'_>> {
        Some(self)
    }
}

impl PointTrait for &PointZ {
    type CoordType<'a>
        = &'a PointZ
    where
        Self: 'a;

    fn coord(&self) -> Option<Self::CoordType<'_>> {
        Some(self)
    }
}

pub struct LineString<'a>(&'a [Point]);

impl LineStringTrait for LineString<'_> {
    type CoordType<'b>
        = &'b Point
    where
        Self: 'b;

    fn num_coords(&self) -> usize {
        self.0.len()
    }

    unsafe fn coord_unchecked(&self, i: usize) -> Self::CoordType<'_> {
        self.0.get_unchecked(i)
    }
}

pub struct LineStringM<'a>(&'a [PointM]);

impl LineStringTrait for LineStringM<'_> {
    type CoordType<'b>
        = &'b PointM
    where
        Self: 'b;

    fn num_coords(&self) -> usize {
        self.0.len()
    }

    unsafe fn coord_unchecked(&self, i: usize) -> Self::CoordType<'_> {
        self.0.get_unchecked(i)
    }
}

pub struct LineStringZ<'a>(&'a [PointZ]);

impl LineStringTrait for LineStringZ<'_> {
    type CoordType<'b>
        = &'b PointZ
    where
        Self: 'b;

    fn num_coords(&self) -> usize {
        self.0.len()
    }

    unsafe fn coord_unchecked(&self, i: usize) -> Self::CoordType<'_> {
        self.0.get_unchecked(i)
    }
}

pub struct Polygon {
    outer: Vec<Point>,
    inner: Vec<Vec<Point>>,
}

impl<'a> PolygonTrait for &'a Polygon {
    type RingType<'b>
        = LineString<'a>
    where
        Self: 'b;

    fn num_interiors(&self) -> usize {
        self.inner.len()
    }

    fn exterior(&self) -> Option<Self::RingType<'_>> {
        Some(LineString(&self.outer))
    }

    unsafe fn interior_unchecked(&self, i: usize) -> Self::RingType<'_> {
        LineString(&self.inner[i])
    }
}

pub struct PolygonM {
    outer: Vec<PointM>,
    inner: Vec<Vec<PointM>>,
}

impl<'a> PolygonTrait for &'a PolygonM {
    type RingType<'b>
        = LineStringM<'a>
    where
        Self: 'b;

    fn num_interiors(&self) -> usize {
        self.inner.len()
    }

    fn exterior(&self) -> Option<Self::RingType<'_>> {
        Some(LineStringM(&self.outer))
    }

    unsafe fn interior_unchecked(&self, i: usize) -> Self::RingType<'_> {
        LineStringM(&self.inner[i])
    }
}

pub struct PolygonZ {
    outer: Vec<PointZ>,
    inner: Vec<Vec<PointZ>>,
}

impl<'a> PolygonTrait for &'a PolygonZ {
    type RingType<'b>
        = LineStringZ<'a>
    where
        Self: 'b;

    fn num_interiors(&self) -> usize {
        self.inner.len()
    }

    fn exterior(&self) -> Option<Self::RingType<'_>> {
        Some(LineStringZ(&self.outer))
    }

    unsafe fn interior_unchecked(&self, i: usize) -> Self::RingType<'_> {
        LineStringZ(&self.inner[i])
    }
}

impl MultiPointTrait for Multipoint {
    type InnerPointType<'b>
        = &'b Point
    where
        Self: 'b;

    fn num_points(&self) -> usize {
        self.points().len()
    }

    unsafe fn point_unchecked(&self, i: usize) -> Self::PointType<'_> {
        self.point(i).unwrap()
    }
}

impl MultiPointTrait for MultipointM {
    type InnerPointType<'b>
        = &'b PointM
    where
        Self: 'b;

    fn num_points(&self) -> usize {
        self.points().len()
    }

    unsafe fn point_unchecked(&self, i: usize) -> Self::PointType<'_> {
        self.point(i).unwrap()
    }
}

impl MultiPointTrait for MultipointZ {
    type InnerPointType<'b>
        = &'b PointZ
    where
        Self: 'b;

    fn num_points(&self) -> usize {
        self.points().len()
    }

    unsafe fn point_unchecked(&self, i: usize) -> Self::PointType<'_> {
        self.point(i).unwrap()
    }
}

impl MultiLineStringTrait for Polyline {
    type InnerLineStringType<'a>
        = LineString<'a>
    where
        Self: 'a;

    fn num_line_strings(&self) -> usize {
        self.parts().len()
    }

    unsafe fn line_string_unchecked(&self, i: usize) -> Self::LineStringType<'_> {
        LineString(self.part(i).unwrap())
    }
}

impl MultiLineStringTrait for PolylineM {
    type InnerLineStringType<'a>
        = LineStringM<'a>
    where
        Self: 'a;

    fn num_line_strings(&self) -> usize {
        self.parts().len()
    }

    unsafe fn line_string_unchecked(&self, i: usize) -> Self::LineStringType<'_> {
        LineStringM(self.part(i).unwrap())
    }
}

impl MultiLineStringTrait for PolylineZ {
    type InnerLineStringType<'a>
        = LineStringZ<'a>
    where
        Self: 'a;

    fn num_line_strings(&self) -> usize {
        self.parts().len()
    }

    unsafe fn line_string_unchecked(&self, i: usize) -> Self::LineStringType<'_> {
        LineStringZ(self.part(i).unwrap())
    }
}

pub struct MultiPolygon(Vec<Polygon>);

impl TryFrom<crate::Polygon> for MultiPolygon {
    type Error = crate::Error;

    fn try_from(geom: crate::Polygon) -> Result<Self, Self::Error> {
        let mut last_poly = None;
        let mut polygons = Vec::new();
        for ring in geom.into_inner() {
            match ring {
                PolygonRing::Outer(points) => {
                    if let Some(poly) = last_poly.take() {
                        polygons.push(poly);
                    }
                    last_poly = Some(Polygon {
                        outer: points,
                        inner: vec![],
                    });
                }
                PolygonRing::Inner(points) => {
                    if let Some(poly) = last_poly.as_mut() {
                        poly.inner.push(points);
                    } else {
                        return Err(crate::Error::OrphanedInnerRing);
                    }
                }
            }
        }

        if let Some(poly) = last_poly.take() {
            polygons.push(poly);
        }

        Ok(Self(polygons))
    }
}

impl MultiPolygonTrait for MultiPolygon {
    type InnerPolygonType<'a> = &'a Polygon;

    fn num_polygons(&self) -> usize {
        self.0.len()
    }

    unsafe fn polygon_unchecked(&self, i: usize) -> Self::PolygonType<'_> {
        &self.0[i]
    }
}

pub struct MultiPolygonM(Vec<PolygonM>);

impl TryFrom<crate::PolygonM> for MultiPolygonM {
    type Error = crate::Error;

    fn try_from(geom: crate::PolygonM) -> Result<Self, Self::Error> {
        let mut last_poly = None;
        let mut polygons = Vec::new();
        for ring in geom.into_inner() {
            match ring {
                PolygonRing::Outer(points) => {
                    if let Some(poly) = last_poly.take() {
                        polygons.push(poly);
                    }
                    last_poly = Some(PolygonM {
                        outer: points,
                        inner: vec![],
                    });
                }
                PolygonRing::Inner(points) => {
                    if let Some(poly) = last_poly.as_mut() {
                        poly.inner.push(points);
                    } else {
                        return Err(crate::Error::OrphanedInnerRing);
                    }
                }
            }
        }

        if let Some(poly) = last_poly.take() {
            polygons.push(poly);
        }

        Ok(Self(polygons))
    }
}

impl MultiPolygonTrait for MultiPolygonM {
    type InnerPolygonType<'a> = &'a PolygonM;

    fn num_polygons(&self) -> usize {
        self.0.len()
    }

    unsafe fn polygon_unchecked(&self, i: usize) -> Self::PolygonType<'_> {
        &self.0[i]
    }
}

pub struct MultiPolygonZ(Vec<PolygonZ>);

impl TryFrom<crate::PolygonZ> for MultiPolygonZ {
    type Error = crate::Error;

    fn try_from(geom: crate::PolygonZ) -> Result<Self, Self::Error> {
        let mut last_poly = None;
        let mut polygons = Vec::new();
        for ring in geom.into_inner() {
            match ring {
                PolygonRing::Outer(points) => {
                    if let Some(poly) = last_poly.take() {
                        polygons.push(poly);
                    }
                    last_poly = Some(PolygonZ {
                        outer: points,
                        inner: vec![],
                    });
                }
                PolygonRing::Inner(points) => {
                    if let Some(poly) = last_poly.as_mut() {
                        poly.inner.push(points);
                    } else {
                        return Err(crate::Error::OrphanedInnerRing);
                    }
                }
            }
        }

        if let Some(poly) = last_poly.take() {
            polygons.push(poly);
        }

        Ok(Self(polygons))
    }
}

impl MultiPolygonTrait for MultiPolygonZ {
    type InnerPolygonType<'a> = &'a PolygonZ;

    fn num_polygons(&self) -> usize {
        self.0.len()
    }

    unsafe fn polygon_unchecked(&self, i: usize) -> Self::PolygonType<'_> {
        &self.0[i]
    }
}

/////////////////////////////////
// Geometry trait implementations

impl GeometryTrait for Point {
    type T = f64;
    type PointType<'b>
        = Self
    where
        Self: 'b;
    type LineStringType<'b>
        = UnimplementedLineString<Self::T>
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xy
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::Point(self)
    }
}

impl GeometryTrait for &Point {
    type T = f64;
    type PointType<'b>
        = Self
    where
        Self: 'b;
    type LineStringType<'b>
        = UnimplementedLineString<Self::T>
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xy
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::Point(self)
    }
}

impl GeometryTrait for PointM {
    type T = f64;
    type PointType<'b>
        = Self
    where
        Self: 'b;
    type LineStringType<'b>
        = UnimplementedLineString<Self::T>
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        if self.m <= NO_DATA {
            geo_traits::Dimensions::Xy
        } else {
            geo_traits::Dimensions::Xym
        }
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::Point(self)
    }
}

impl GeometryTrait for &PointM {
    type T = f64;
    type PointType<'b>
        = Self
    where
        Self: 'b;
    type LineStringType<'b>
        = UnimplementedLineString<Self::T>
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        if self.m <= NO_DATA {
            geo_traits::Dimensions::Xy
        } else {
            geo_traits::Dimensions::Xym
        }
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::Point(self)
    }
}

impl GeometryTrait for PointZ {
    type T = f64;
    type PointType<'b>
        = Self
    where
        Self: 'b;
    type LineStringType<'b>
        = UnimplementedLineString<Self::T>
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        if self.m <= NO_DATA {
            geo_traits::Dimensions::Xyz
        } else {
            geo_traits::Dimensions::Xyzm
        }
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::Point(self)
    }
}

impl GeometryTrait for &PointZ {
    type T = f64;
    type PointType<'b>
        = Self
    where
        Self: 'b;
    type LineStringType<'b>
        = UnimplementedLineString<Self::T>
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        if self.m <= NO_DATA {
            geo_traits::Dimensions::Xyz
        } else {
            geo_traits::Dimensions::Xyzm
        }
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::Point(self)
    }
}

impl<'a> GeometryTrait for LineString<'a> {
    type T = f64;
    type PointType<'b>
        = UnimplementedPoint<Self::T>
    where
        Self: 'b;
    type LineStringType<'b>
        = Self
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xy
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::LineString(self)
    }
}

impl<'a> GeometryTrait for LineStringM<'a> {
    type T = f64;
    type PointType<'b>
        = UnimplementedPoint<Self::T>
    where
        Self: 'b;
    type LineStringType<'b>
        = Self
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xym
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::LineString(self)
    }
}

impl<'a> GeometryTrait for LineStringZ<'a> {
    type T = f64;
    type PointType<'b>
        = UnimplementedPoint<Self::T>
    where
        Self: 'b;
    type LineStringType<'b>
        = Self
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        // Check the first underlying coordinate to check if it's XYZ or XYZM
        self.0
            .first()
            .map(CoordTrait::dim)
            .unwrap_or(geo_traits::Dimensions::Xyz)
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::LineString(self)
    }
}

impl GeometryTrait for &'_ Polygon {
    type T = f64;
    type PointType<'b>
        = UnimplementedPoint<Self::T>
    where
        Self: 'b;
    type LineStringType<'b>
        = UnimplementedLineString<Self::T>
    where
        Self: 'b;
    type PolygonType<'b>
        = Self
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xy
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::Polygon(self)
    }
}

impl GeometryTrait for &'_ PolygonM {
    type T = f64;
    type PointType<'b>
        = UnimplementedPoint<Self::T>
    where
        Self: 'b;
    type LineStringType<'b>
        = UnimplementedLineString<Self::T>
    where
        Self: 'b;
    type PolygonType<'b>
        = Self
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xym
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::Polygon(self)
    }
}

impl GeometryTrait for &'_ PolygonZ {
    type T = f64;
    type PointType<'b>
        = UnimplementedPoint<Self::T>
    where
        Self: 'b;
    type LineStringType<'b>
        = UnimplementedLineString<Self::T>
    where
        Self: 'b;
    type PolygonType<'b>
        = Self
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        // Check the first coord of the outer ring to check if it's XYZ or XYZM
        self.outer
            .first()
            .map(CoordTrait::dim)
            .unwrap_or(geo_traits::Dimensions::Xyz)
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::Polygon(self)
    }
}

impl GeometryTrait for Multipoint {
    type T = f64;
    type PointType<'b>
        = &'b Point
    where
        Self: 'b;
    type LineStringType<'b>
        = UnimplementedLineString<Self::T>
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = Self
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xy
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::MultiPoint(self)
    }
}

impl GeometryTrait for MultipointM {
    type T = f64;
    type PointType<'b>
        = &'b PointM
    where
        Self: 'b;
    type LineStringType<'b>
        = UnimplementedLineString<Self::T>
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = Self
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xym
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::MultiPoint(self)
    }
}

impl GeometryTrait for MultipointZ {
    type T = f64;
    type PointType<'b>
        = &'b PointZ
    where
        Self: 'b;
    type LineStringType<'b>
        = UnimplementedLineString<Self::T>
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = Self
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        // Check the first point to check if it's XYZ or XYZM
        self.points
            .first()
            .map(CoordTrait::dim)
            .unwrap_or(geo_traits::Dimensions::Xyz)
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::MultiPoint(self)
    }
}

impl GeometryTrait for Polyline {
    type T = f64;
    type PointType<'b>
        = &'b Point
    where
        Self: 'b;
    type LineStringType<'b>
        = LineString<'b>
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = Self
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xy
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::MultiLineString(self)
    }
}

impl GeometryTrait for PolylineM {
    type T = f64;
    type PointType<'b>
        = &'b PointM
    where
        Self: 'b;
    type LineStringType<'b>
        = LineStringM<'b>
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = Self
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xym
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::MultiLineString(self)
    }
}

impl GeometryTrait for PolylineZ {
    type T = f64;
    type PointType<'b>
        = &'b PointZ
    where
        Self: 'b;
    type LineStringType<'b>
        = LineStringZ<'b>
    where
        Self: 'b;
    type PolygonType<'b>
        = UnimplementedPolygon<Self::T>
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = Self
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = UnimplementedMultiPolygon<Self::T>
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        // Check the first point to check if it's XYZ or XYZM
        self.parts
            .first()
            .and_then(|line_string| line_string.first().map(CoordTrait::dim))
            .unwrap_or(geo_traits::Dimensions::Xyz)
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::MultiLineString(self)
    }
}

impl GeometryTrait for MultiPolygon {
    type T = f64;
    type PointType<'b>
        = &'b Point
    where
        Self: 'b;
    type LineStringType<'b>
        = LineString<'b>
    where
        Self: 'b;
    type PolygonType<'b>
        = &'b Polygon
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = Self
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xy
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::MultiPolygon(self)
    }
}

impl GeometryTrait for MultiPolygonM {
    type T = f64;
    type PointType<'b>
        = &'b PointM
    where
        Self: 'b;
    type LineStringType<'b>
        = LineStringM<'b>
    where
        Self: 'b;
    type PolygonType<'b>
        = &'b PolygonM
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = Self
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        geo_traits::Dimensions::Xym
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::MultiPolygon(self)
    }
}

impl GeometryTrait for MultiPolygonZ {
    type T = f64;
    type PointType<'b>
        = &'b PointZ
    where
        Self: 'b;
    type LineStringType<'b>
        = LineStringZ<'b>
    where
        Self: 'b;
    type PolygonType<'b>
        = &'b PolygonZ
    where
        Self: 'b;
    type MultiPointType<'b>
        = UnimplementedMultiPoint<Self::T>
    where
        Self: 'b;
    type MultiLineStringType<'b>
        = UnimplementedMultiLineString<Self::T>
    where
        Self: 'b;
    type MultiPolygonType<'b>
        = Self
    where
        Self: 'b;
    type GeometryCollectionType<'b>
        = geo_traits::UnimplementedGeometryCollection<Self::T>
    where
        Self: 'b;
    type RectType<'b>
        = geo_traits::UnimplementedRect<f64>
    where
        Self: 'b;
    type LineType<'b>
        = geo_traits::UnimplementedLine<f64>
    where
        Self: 'b;
    type TriangleType<'b>
        = geo_traits::UnimplementedTriangle<f64>
    where
        Self: 'b;

    fn dim(&self) -> geo_traits::Dimensions {
        // Check the first polygon to check if it's XYZ or XYZM
        self.0
            .first()
            .map(|polygon| polygon.dim())
            .unwrap_or(geo_traits::Dimensions::Xyz)
    }

    fn as_type(
        &self,
    ) -> geo_traits::GeometryType<
        '_,
        Self::PointType<'_>,
        Self::LineStringType<'_>,
        Self::PolygonType<'_>,
        Self::MultiPointType<'_>,
        Self::MultiLineStringType<'_>,
        Self::MultiPolygonType<'_>,
        Self::GeometryCollectionType<'_>,
        Self::RectType<'_>,
        Self::TriangleType<'_>,
        Self::LineType<'_>,
    > {
        geo_traits::GeometryType::MultiPolygon(self)
    }
}
