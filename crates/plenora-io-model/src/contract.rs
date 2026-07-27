//! `DataContract` e affini (scheletro Fase 0). La v1 ammette al massimo una
//! colonna geometria (Architetture §2.2, D16 data-tools).

use std::collections::BTreeMap;

use arrow_schema::SchemaRef;
use serde::Serialize;

use crate::crs::{CrsResolution, ResolvedCrs};

/// Identità logica stabile di un campo nel grafo (namespace globale).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FieldId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryEncoding {
    Wkb,
    Ewkb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CoordinateDimensions {
    Xy,
    Xyz,
    Xym,
    Xyzm,
    /// Il driver preserva i byte ma non ha ancora risolto la dimensionalità.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpatialSemantics {
    Geometry,
    Geography,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinatePrecision {
    Float64,
    Float32,
    /// Precisione delegata al formato/database e descritta in
    /// `native_metadata`.
    Native,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GeometryType {
    Point,
    LineString,
    Polygon,
    MultiPoint,
    MultiLineString,
    MultiPolygon,
    GeometryCollection,
}

impl GeometryType {
    /// Nome wire canonico ratificato da R3.1.
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::LineString => "linestring",
            Self::Polygon => "polygon",
            Self::MultiPoint => "multipoint",
            Self::MultiLineString => "multilinestring",
            Self::MultiPolygon => "multipolygon",
            Self::GeometryCollection => "geometrycollection",
        }
    }

    /// Risolve esclusivamente un nome wire canonico supportato.
    pub fn from_canonical_name(value: &str) -> Option<Self> {
        match value {
            "point" => Some(Self::Point),
            "linestring" => Some(Self::LineString),
            "polygon" => Some(Self::Polygon),
            "multipoint" => Some(Self::MultiPoint),
            "multilinestring" => Some(Self::MultiLineString),
            "multipolygon" => Some(Self::MultiPolygon),
            "geometrycollection" => Some(Self::GeometryCollection),
            _ => None,
        }
    }
}

/// Contratto di una colonna geometrica.
#[derive(Clone, Debug)]
pub struct GeometryColumnContract {
    pub field_id: FieldId,
    pub name: String,
    pub crs: CrsResolution,
    pub nullable: bool,
    pub encoding: GeometryEncoding,
    pub dimensions: CoordinateDimensions,
    pub spatial_semantics: SpatialSemantics,
    /// SRID nativo quando è distinto o aggiuntivo rispetto al CRS risolto.
    pub srid: Option<i32>,
    pub precision: CoordinatePrecision,
    /// Tipi noti staticamente. Vuoto significa non ancora determinato; non
    /// equivale a "nessuna geometria".
    pub geometry_types: Vec<GeometryType>,
    /// Metadati nativi namespaced, per esempio `postgis.typmod`,
    /// `gpkg.geometry_type_name`, `sql.type_name`.
    pub native_metadata: BTreeMap<String, String>,
}

impl GeometryColumnContract {
    pub fn wkb_xy(
        field_id: FieldId,
        name: impl Into<String>,
        crs: impl Into<CrsResolution>,
        nullable: bool,
    ) -> Self {
        Self {
            field_id,
            name: name.into(),
            crs: crs.into(),
            nullable,
            encoding: GeometryEncoding::Wkb,
            dimensions: CoordinateDimensions::Xy,
            spatial_semantics: SpatialSemantics::Geometry,
            srid: None,
            precision: CoordinatePrecision::Float64,
            geometry_types: Vec::new(),
            native_metadata: BTreeMap::new(),
        }
    }

    pub fn wkb_passthrough(
        field_id: FieldId,
        name: impl Into<String>,
        crs: impl Into<CrsResolution>,
        nullable: bool,
    ) -> Self {
        Self {
            dimensions: CoordinateDimensions::Unknown,
            ..Self::wkb_xy(field_id, name, crs, nullable)
        }
    }

    pub fn resolved_crs(&self) -> Option<&ResolvedCrs> {
        self.crs.as_resolved()
    }
}

/// Contratto dei dati che attraversano un arco / un layer.
#[derive(Clone, Debug)]
pub struct DataContract {
    pub schema: SchemaRef,
    /// v1: al massimo una colonna geometria.
    pub geometry: Option<GeometryColumnContract>,
}

/// Contratto di un layer di un dataset aperto.
#[derive(Clone, Debug)]
pub struct LayerContract {
    pub id: LayerId,
    pub name: String,
    pub contract: DataContract,
}

#[cfg(test)]
mod tests {
    use super::{CoordinateDimensions, GeometryType};

    #[test]
    fn geometry_type_wire_names_match_ratified_r3_1() {
        let values = [
            GeometryType::Point,
            GeometryType::LineString,
            GeometryType::Polygon,
            GeometryType::MultiPoint,
            GeometryType::MultiLineString,
            GeometryType::MultiPolygon,
            GeometryType::GeometryCollection,
        ];

        assert_eq!(
            serde_json::to_string(&values).unwrap(),
            r#"["point","linestring","polygon","multipoint","multilinestring","multipolygon","geometrycollection"]"#
        );
        for value in values {
            assert_eq!(
                GeometryType::from_canonical_name(value.canonical_name()),
                Some(value)
            );
        }
    }

    #[test]
    fn coordinate_dimension_wire_names_are_explicitly_lowercase() {
        let values = [
            CoordinateDimensions::Xy,
            CoordinateDimensions::Xyz,
            CoordinateDimensions::Xym,
            CoordinateDimensions::Xyzm,
            CoordinateDimensions::Unknown,
        ];

        assert_eq!(
            serde_json::to_string(&values).unwrap(),
            r#"["xy","xyz","xym","xyzm","unknown"]"#
        );
    }
}
