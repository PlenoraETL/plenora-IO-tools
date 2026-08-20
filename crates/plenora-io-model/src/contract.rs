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
    CircularString,
    CompoundCurve,
    CurvePolygon,
    MultiCurve,
    MultiSurface,
    PolyhedralSurface,
    Tin,
    Triangle,
    Unknown,
}

impl GeometryEncoding {
    /// Nome statico dell'encoding, per i messaggi pubblici.
    #[must_use]
    pub const fn nome(self) -> &'static str {
        match self {
            Self::Wkb => "wkb",
            Self::Ewkb => "ewkb",
        }
    }
}

impl CoordinateDimensions {
    /// Nome statico della dimensionalita', per i messaggi pubblici.
    #[must_use]
    pub const fn nome(self) -> &'static str {
        match self {
            Self::Xy => "xy",
            Self::Xyz => "xyz",
            Self::Xym => "xym",
            Self::Xyzm => "xyzm",
            Self::Unknown => "unknown",
        }
    }
}

impl SpatialSemantics {
    /// Nome statico della semantica spaziale, per i messaggi pubblici.
    #[must_use]
    pub const fn nome(self) -> &'static str {
        match self {
            Self::Geometry => "geometry",
            Self::Geography => "geography",
        }
    }
}

impl GeometryType {
    /// Nome wire canonico ratificato da R3.1.
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::LineString => "linestring",
            Self::Polygon => "polygon",
            Self::MultiPoint => "multipoint",
            Self::MultiLineString => "multilinestring",
            Self::MultiPolygon => "multipolygon",
            Self::GeometryCollection => "geometrycollection",
            Self::CircularString => "circularstring",
            Self::CompoundCurve => "compoundcurve",
            Self::CurvePolygon => "curvepolygon",
            Self::MultiCurve => "multicurve",
            Self::MultiSurface => "multisurface",
            Self::PolyhedralSurface => "polyhedralsurface",
            Self::Tin => "tin",
            Self::Triangle => "triangle",
            Self::Unknown => "unknown",
        }
    }

    /// Risolve esclusivamente un nome wire canonico supportato.
    #[must_use]
    pub fn from_canonical_name(value: &str) -> Option<Self> {
        match value {
            "point" => Some(Self::Point),
            "linestring" => Some(Self::LineString),
            "polygon" => Some(Self::Polygon),
            "multipoint" => Some(Self::MultiPoint),
            "multilinestring" => Some(Self::MultiLineString),
            "multipolygon" => Some(Self::MultiPolygon),
            "geometrycollection" => Some(Self::GeometryCollection),
            "circularstring" => Some(Self::CircularString),
            "compoundcurve" => Some(Self::CompoundCurve),
            "curvepolygon" => Some(Self::CurvePolygon),
            "multicurve" => Some(Self::MultiCurve),
            "multisurface" => Some(Self::MultiSurface),
            "polyhedralsurface" => Some(Self::PolyhedralSurface),
            "tin" => Some(Self::Tin),
            "triangle" => Some(Self::Triangle),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TypesDeclaration {
    Exact,
    Mixed,
    Unresolved,
    /// Ingresso legacy nel quale la proprietà non era dichiarata. Non ha un
    /// valore wire: va preservato come assenza, mai emesso come `unresolved`.
    #[serde(skip)]
    LegacyUndeclared,
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
    pub types_declaration: TypesDeclaration,
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
            types_declaration: TypesDeclaration::Unresolved,
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

    #[must_use]
    pub const fn resolved_crs(&self) -> Option<&ResolvedCrs> {
        self.crs.as_resolved()
    }

    pub fn set_exact_geometry_types(&mut self, geometry_types: Vec<GeometryType>) {
        self.geometry_types = geometry_types;
        self.geometry_types.sort_unstable();
        self.geometry_types.dedup();
        self.types_declaration = if self.geometry_types.is_empty() {
            TypesDeclaration::Unresolved
        } else {
            TypesDeclaration::Exact
        };
    }
}

/// Contratto dei dati che attraversano un arco / un layer.
#[derive(Clone, Debug)]
pub struct DataContract {
    pub schema: SchemaRef,
    /// v1: al massimo una colonna geometria.
    pub geometry: Option<GeometryColumnContract>,
}

impl DataContract {
    /// Costruisce il contratto e inserisce la versione del protocollo nei
    /// metadati di schema quando sono presenti chiavi geometriche canoniche.
    #[must_use]
    // Firma per valore: `DataContract::new` e' parte dell'identita' pubblica
    // versionata; passare per riferimento la cambierebbe senza guadagno.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(schema: SchemaRef, geometry: Option<GeometryColumnContract>) -> Self {
        let schema = geometry.as_ref().map_or_else(
            || schema.clone(),
            |geometry| {
                if geometry.types_declaration == TypesDeclaration::LegacyUndeclared {
                    return schema.clone();
                }
                let fields = schema
                    .fields()
                    .iter()
                    .map(|field| {
                        if field.name() == &geometry.name {
                            crate::geometry::with_geometry_contract_metadata(field, geometry)
                        } else {
                            field.as_ref().clone()
                        }
                    })
                    .collect::<Vec<_>>();
                crate::geometry::with_contract_version(std::sync::Arc::new(
                    arrow_schema::Schema::new_with_metadata(fields, schema.metadata().clone()),
                ))
            },
        );
        Self { schema, geometry }
    }
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
            GeometryType::CircularString,
            GeometryType::CompoundCurve,
            GeometryType::CurvePolygon,
            GeometryType::MultiCurve,
            GeometryType::MultiSurface,
            GeometryType::PolyhedralSurface,
            GeometryType::Tin,
            GeometryType::Triangle,
            GeometryType::Unknown,
        ];

        assert_eq!(
            serde_json::to_string(&values).unwrap(),
            r#"["point","linestring","polygon","multipoint","multilinestring","multipolygon","geometrycollection","circularstring","compoundcurve","curvepolygon","multicurve","multisurface","polyhedralsurface","tin","triangle","unknown"]"#
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
