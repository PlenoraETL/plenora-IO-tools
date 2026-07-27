//! Convenzione della colonna geometria nel bordo Arrow (GeoArrow-WKB).

use crate::contract::{
    CoordinateDimensions, CoordinatePrecision, GeometryColumnContract, GeometryEncoding,
    GeometryType, SpatialSemantics,
};
use crate::{PlenoraIoError, Result};

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

/// Indica quali parti opzionali del contratto erano realmente dichiarate nel
/// campo Arrow. Serve a distinguere un default legacy da un valore esplicito.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GeometryMetadataPresence {
    dimensions: bool,
}

impl GeometryMetadataPresence {
    pub fn has_dimensions(self) -> bool {
        self.dimensions
    }
}

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
            .map(|geometry_type| geometry_type.canonical_name())
            .collect::<Vec<_>>()
            .join(","),
    );
    for (key, value) in &contract.native_metadata {
        metadata.insert(format!("{PLENORA_NATIVE_PREFIX}{key}"), value.clone());
    }
    field.clone().with_metadata(metadata)
}

fn invalid_metadata(field: &arrow_schema::Field, key: &str) -> PlenoraIoError {
    PlenoraIoError::Contract(format!(
        "campo geometria '{}': metadato {key} non valido",
        field.name()
    ))
}

/// Applica al contratto base gli eventuali metadati namespaced del campo.
///
/// I metadati assenti non modificano il contratto base; i metadati presenti ma
/// non riconosciuti sono un errore di contratto. Questo evita di trasformare
/// input corrotti o futuri in valori di comodo.
pub fn read_geometry_contract_metadata(
    field: &arrow_schema::Field,
    contract: &mut GeometryColumnContract,
) -> Result<GeometryMetadataPresence> {
    let metadata = field.metadata();
    let mut parsed = contract.clone();
    let mut presence = GeometryMetadataPresence::default();

    if let Some(value) = metadata.get(PLENORA_ENCODING_KEY) {
        parsed.encoding = match value.as_str() {
            "wkb" => GeometryEncoding::Wkb,
            "ewkb" => GeometryEncoding::Ewkb,
            _ => return Err(invalid_metadata(field, PLENORA_ENCODING_KEY)),
        };
    }
    if let Some(value) = metadata.get(PLENORA_DIMENSIONS_KEY) {
        presence.dimensions = true;
        parsed.dimensions = match value.as_str() {
            "xy" => CoordinateDimensions::Xy,
            "xyz" => CoordinateDimensions::Xyz,
            "xym" => CoordinateDimensions::Xym,
            "xyzm" => CoordinateDimensions::Xyzm,
            "unknown" => CoordinateDimensions::Unknown,
            _ => return Err(invalid_metadata(field, PLENORA_DIMENSIONS_KEY)),
        };
    }
    if let Some(value) = metadata.get(PLENORA_SPATIAL_SEMANTICS_KEY) {
        parsed.spatial_semantics = match value.as_str() {
            "geometry" => SpatialSemantics::Geometry,
            "geography" => SpatialSemantics::Geography,
            _ => return Err(invalid_metadata(field, PLENORA_SPATIAL_SEMANTICS_KEY)),
        };
    }
    if let Some(value) = metadata.get(PLENORA_PRECISION_KEY) {
        parsed.precision = match value.as_str() {
            "float64" => CoordinatePrecision::Float64,
            "float32" => CoordinatePrecision::Float32,
            "native" => CoordinatePrecision::Native,
            _ => return Err(invalid_metadata(field, PLENORA_PRECISION_KEY)),
        };
    }
    if let Some(value) = metadata.get(PLENORA_SRID_KEY) {
        parsed.srid = Some(
            value
                .parse()
                .map_err(|_| invalid_metadata(field, PLENORA_SRID_KEY))?,
        );
    }
    if let Some(value) = metadata.get(PLENORA_GEOMETRY_TYPES_KEY) {
        let mut geometry_types = Vec::new();
        if !value.is_empty() {
            for value in value.split(',') {
                let geometry_type = GeometryType::from_canonical_name(value)
                    .ok_or_else(|| invalid_metadata(field, PLENORA_GEOMETRY_TYPES_KEY))?;
                if geometry_types.contains(&geometry_type) {
                    return Err(invalid_metadata(field, PLENORA_GEOMETRY_TYPES_KEY));
                }
                geometry_types.push(geometry_type);
            }
        }
        parsed.geometry_types = geometry_types;
    }
    for (key, value) in metadata {
        if let Some(key) = key.strip_prefix(PLENORA_NATIVE_PREFIX) {
            if key.is_empty() {
                return Err(invalid_metadata(field, PLENORA_NATIVE_PREFIX));
            }
            parsed.native_metadata.insert(key.to_owned(), value.clone());
        }
    }

    *contract = parsed;
    Ok(presence)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use arrow_schema::{DataType, Field};

    use super::*;
    use crate::contract::FieldId;
    use crate::crs::CrsResolution;

    fn contract() -> GeometryColumnContract {
        GeometryColumnContract::wkb_xy(FieldId(0), "geometry", CrsResolution::Missing, true)
    }

    fn field_with(key: &str, value: &str) -> Field {
        Field::new("geometry", DataType::Binary, true)
            .with_metadata(HashMap::from([(key.to_owned(), value.to_owned())]))
    }

    #[test]
    fn missing_dimensions_preserve_base_and_report_absence() {
        let field = Field::new("geometry", DataType::Binary, true);
        let mut geometry = contract();

        let presence = read_geometry_contract_metadata(&field, &mut geometry).unwrap();

        assert!(!presence.has_dimensions());
        assert_eq!(geometry.dimensions, CoordinateDimensions::Xy);
    }

    #[test]
    fn explicit_unknown_dimensions_are_preserved() {
        let field = field_with(PLENORA_DIMENSIONS_KEY, "unknown");
        let mut geometry = contract();

        let presence = read_geometry_contract_metadata(&field, &mut geometry).unwrap();

        assert!(presence.has_dimensions());
        assert_eq!(geometry.dimensions, CoordinateDimensions::Unknown);
    }

    #[test]
    fn invalid_metadata_is_rejected_without_partial_mutation() {
        let field = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([
            (PLENORA_ENCODING_KEY.to_owned(), "ewkb".to_owned()),
            (PLENORA_PRECISION_KEY.to_owned(), "binary128".to_owned()),
        ]));
        let mut geometry = contract();
        let original = geometry.clone();

        assert!(matches!(
            read_geometry_contract_metadata(&field, &mut geometry),
            Err(PlenoraIoError::Contract(_))
        ));
        assert_eq!(geometry.encoding, original.encoding);
        assert_eq!(geometry.precision, original.precision);
    }

    #[test]
    fn invalid_srid_and_geometry_types_are_rejected() {
        for field in [
            field_with(PLENORA_SRID_KEY, "not-an-i32"),
            field_with(PLENORA_GEOMETRY_TYPES_KEY, "point,futuregeometry"),
            field_with(PLENORA_GEOMETRY_TYPES_KEY, "point,point"),
            field_with(PLENORA_GEOMETRY_TYPES_KEY, "line_string"),
            field_with(PLENORA_GEOMETRY_TYPES_KEY, "LineString"),
        ] {
            assert!(matches!(
                read_geometry_contract_metadata(&field, &mut contract()),
                Err(PlenoraIoError::Contract(_))
            ));
        }
    }

    #[test]
    fn geometry_type_metadata_uses_canonical_names_without_separators() {
        let mut geometry = contract();
        geometry.geometry_types = vec![
            GeometryType::LineString,
            GeometryType::MultiPolygon,
            GeometryType::GeometryCollection,
        ];

        let field = with_geometry_contract_metadata(
            &Field::new("geometry", DataType::Binary, true),
            &geometry,
        );

        assert_eq!(
            field
                .metadata()
                .get(PLENORA_GEOMETRY_TYPES_KEY)
                .map(String::as_str),
            Some("linestring,multipolygon,geometrycollection")
        );

        let mut decoded = contract();
        read_geometry_contract_metadata(&field, &mut decoded).unwrap();
        assert_eq!(decoded.geometry_types, geometry.geometry_types);
    }
}
