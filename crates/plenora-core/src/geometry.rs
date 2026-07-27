//! Convenzione della colonna geometria nel bordo Arrow (GeoArrow-WKB).

use crate::contract::{
    CoordinateDimensions, CoordinatePrecision, GeometryColumnContract, GeometryEncoding,
    GeometryType, SpatialSemantics,
};

/// Nome dell'estensione GeoArrow per una colonna geometria WKB.
pub const GEOARROW_WKB_EXTENSION: &str = "geoarrow.wkb";
/// Chiave dei metadati di estensione a livello di campo Arrow.
pub const ARROW_EXTENSION_NAME_KEY: &str = "ARROW:extension:name";
/// Chiave del metadato `geo` (a livello di campo/schema) che porta il CRS.
pub const GEO_METADATA_KEY: &str = "geo";
/// Chiave `crs` (PROJJSON) dentro il metadato `geo`.
pub const GEO_CRS_KEY: &str = "crs";
pub const PLENORA_ENCODING_KEY: &str = "plenora.geometry.encoding";
pub const PLENORA_DIMENSIONS_KEY: &str = "plenora.geometry.dimensions";
pub const PLENORA_SPATIAL_SEMANTICS_KEY: &str = "plenora.geometry.spatial_semantics";
pub const PLENORA_SRID_KEY: &str = "plenora.geometry.srid";
pub const PLENORA_PRECISION_KEY: &str = "plenora.geometry.precision";
pub const PLENORA_GEOMETRY_TYPES_KEY: &str = "plenora.geometry.types";
pub const PLENORA_NATIVE_PREFIX: &str = "plenora.geometry.native.";

/// True se il campo Arrow è marcato come colonna geometria `geoarrow.wkb`.
pub fn is_geometry_field(field: &arrow_schema::Field) -> bool {
    field
        .metadata()
        .get(ARROW_EXTENSION_NAME_KEY)
        .map(|v| v == GEOARROW_WKB_EXTENSION)
        .unwrap_or(false)
}

/// Registra nel campo Arrow le parti del contratto che GeoArrow-WKB da solo
/// non esprime. Le chiavi sono namespaced e quindi sopravvivono in Arrow IPC.
pub fn with_geometry_contract_metadata(
    field: &arrow_schema::Field,
    contract: &GeometryColumnContract,
) -> arrow_schema::Field {
    let mut metadata = field.metadata().clone();
    metadata.insert(
        PLENORA_ENCODING_KEY.to_owned(),
        match contract.encoding {
            GeometryEncoding::Wkb => "wkb",
            GeometryEncoding::Ewkb => "ewkb",
        }
        .to_owned(),
    );
    metadata.insert(
        PLENORA_DIMENSIONS_KEY.to_owned(),
        match contract.dimensions {
            CoordinateDimensions::Xy => "xy",
            CoordinateDimensions::Xyz => "xyz",
            CoordinateDimensions::Xym => "xym",
            CoordinateDimensions::Xyzm => "xyzm",
            CoordinateDimensions::Unknown => "unknown",
        }
        .to_owned(),
    );
    metadata.insert(
        PLENORA_SPATIAL_SEMANTICS_KEY.to_owned(),
        match contract.spatial_semantics {
            SpatialSemantics::Geometry => "geometry",
            SpatialSemantics::Geography => "geography",
        }
        .to_owned(),
    );
    metadata.insert(
        PLENORA_PRECISION_KEY.to_owned(),
        match contract.precision {
            CoordinatePrecision::Float64 => "float64",
            CoordinatePrecision::Float32 => "float32",
            CoordinatePrecision::Native => "native",
        }
        .to_owned(),
    );
    if let Some(srid) = contract.srid {
        metadata.insert(PLENORA_SRID_KEY.to_owned(), srid.to_string());
    } else {
        metadata.remove(PLENORA_SRID_KEY);
    }
    metadata.insert(
        PLENORA_GEOMETRY_TYPES_KEY.to_owned(),
        contract
            .geometry_types
            .iter()
            .map(|geometry_type| format!("{geometry_type:?}"))
            .collect::<Vec<_>>()
            .join(","),
    );
    for (key, value) in &contract.native_metadata {
        metadata.insert(format!("{PLENORA_NATIVE_PREFIX}{key}"), value.clone());
    }
    field.clone().with_metadata(metadata)
}

/// Applica al contratto base gli eventuali metadati namespaced del campo.
pub fn read_geometry_contract_metadata(
    field: &arrow_schema::Field,
    contract: &mut GeometryColumnContract,
) {
    let metadata = field.metadata();
    contract.encoding = match metadata.get(PLENORA_ENCODING_KEY).map(String::as_str) {
        Some("ewkb") => GeometryEncoding::Ewkb,
        _ => GeometryEncoding::Wkb,
    };
    contract.dimensions = match metadata.get(PLENORA_DIMENSIONS_KEY).map(String::as_str) {
        Some("xy") => CoordinateDimensions::Xy,
        Some("xyz") => CoordinateDimensions::Xyz,
        Some("xym") => CoordinateDimensions::Xym,
        Some("xyzm") => CoordinateDimensions::Xyzm,
        _ => CoordinateDimensions::Unknown,
    };
    contract.spatial_semantics = match metadata
        .get(PLENORA_SPATIAL_SEMANTICS_KEY)
        .map(String::as_str)
    {
        Some("geography") => SpatialSemantics::Geography,
        _ => SpatialSemantics::Geometry,
    };
    contract.precision = match metadata.get(PLENORA_PRECISION_KEY).map(String::as_str) {
        Some("float32") => CoordinatePrecision::Float32,
        Some("native") => CoordinatePrecision::Native,
        _ => CoordinatePrecision::Float64,
    };
    contract.srid = metadata
        .get(PLENORA_SRID_KEY)
        .and_then(|value| value.parse().ok());
    contract.geometry_types = metadata
        .get(PLENORA_GEOMETRY_TYPES_KEY)
        .into_iter()
        .flat_map(|value| value.split(','))
        .filter_map(|value| match value {
            "Point" => Some(GeometryType::Point),
            "LineString" => Some(GeometryType::LineString),
            "Polygon" => Some(GeometryType::Polygon),
            "MultiPoint" => Some(GeometryType::MultiPoint),
            "MultiLineString" => Some(GeometryType::MultiLineString),
            "MultiPolygon" => Some(GeometryType::MultiPolygon),
            "GeometryCollection" => Some(GeometryType::GeometryCollection),
            _ => None,
        })
        .collect();
    contract.native_metadata = metadata
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix(PLENORA_NATIVE_PREFIX)
                .map(|key| (key.to_owned(), value.clone()))
        })
        .collect();
}
