//! Convenzione della colonna geometria nel bordo Arrow (GeoArrow-WKB).

use crate::contract::{
    CoordinateDimensions, CoordinatePrecision, GeometryColumnContract, GeometryEncoding,
    GeometryType, SpatialSemantics, TypesDeclaration,
};
use crate::crs::{
    definition_format, AxisOrder, CrsDefinitionFormat, CrsResolution, RawCrs, ResolvedCrs,
};
use crate::{PlenoraIoError, Result};
use std::sync::Arc;

/// Nome dell'estensione GeoArrow per una colonna geometria WKB.
pub const GEOARROW_WKB_EXTENSION: &str = "geoarrow.wkb";
/// Chiave dei metadati di estensione a livello di campo Arrow.
pub const ARROW_EXTENSION_NAME_KEY: &str = "ARROW:extension:name";
/// Chiave del metadato `geo` (a livello di campo/schema) che porta il CRS.
pub const GEO_METADATA_KEY: &str = "geo";
/// Chiave `crs` (PROJJSON) dentro il metadato `geo`.
pub const GEO_CRS_KEY: &str = "crs";
pub const PLENORA_FIELD_ID_KEY: &str = "plenora.field_id";
pub const PLENORA_ENCODING_KEY: &str = "plenora.geometry.encoding";
pub const PLENORA_DIMENSIONS_KEY: &str = "plenora.geometry.dimensions";
pub const PLENORA_SPATIAL_SEMANTICS_KEY: &str = "plenora.geometry.spatial_semantics";
pub const PLENORA_SRID_KEY: &str = "plenora.geometry.srid";
pub const PLENORA_PRECISION_KEY: &str = "plenora.geometry.precision";
pub const PLENORA_GEOMETRY_TYPES_KEY: &str = "plenora.geometry.types";
pub const PLENORA_TYPES_DECLARATION_KEY: &str = "plenora.geometry.types_declaration";
pub const PLENORA_CRS_ID_KEY: &str = "plenora.geometry.crs_id";
pub const PLENORA_CRS_RESOLUTION_KEY: &str = "plenora.geometry.crs_resolution";
pub const PLENORA_CRS_DEFINITION_KEY: &str = "plenora.geometry.crs_definition";
pub const PLENORA_CRS_DEFINITION_FORMAT_KEY: &str = "plenora.geometry.crs_definition_format";
pub const PLENORA_AXIS_ORDER_KEY: &str = "plenora.geometry.axis_order";
pub const PLENORA_CONTRACT_VERSION_KEY: &str = "plenora.contract.version";
pub const PLENORA_CONTRACT_VERSION: &str = "1";
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

/// True se il campo Arrow è riconoscibile come geometria tramite GeoArrow o
/// tramite almeno una chiave canonica Plenora.
///
/// Il riconoscimento è intenzionalmente più largo della validazione: un campo
/// con metadati canonici incompleti deve essere riconosciuto e poi rifiutato,
/// non degradato a binario opaco.
pub fn is_geometry_field(field: &arrow_schema::Field) -> bool {
    is_geoarrow_wkb_field(field) || has_canonical_geometry_metadata(field)
}

fn is_geoarrow_wkb_field(field: &arrow_schema::Field) -> bool {
    field
        .metadata()
        .get(ARROW_EXTENSION_NAME_KEY)
        .map(|v| v == GEOARROW_WKB_EXTENSION)
        .unwrap_or(false)
}

/// Indica se il campo porta una qualunque chiave canonica della geometria.
#[must_use]
pub fn has_canonical_geometry_metadata(field: &arrow_schema::Field) -> bool {
    field
        .metadata()
        .keys()
        .any(|key| key.starts_with("plenora.geometry."))
}

/// Verifica la coerenza dell'identità geometrica secondo R2.6/R2.8.
///
/// Le quattro chiavi obbligatorie del campo devono essere presenti insieme;
/// la versione canonica vive invece sullo schema ed è comunicata dal caller.
pub fn validate_geometry_field_identity(
    field: &arrow_schema::Field,
    schema_contract_version_present: bool,
) -> Result<()> {
    let canonical = has_canonical_geometry_metadata(field);
    let extension = field.metadata().get(ARROW_EXTENSION_NAME_KEY);

    if canonical {
        if !schema_contract_version_present {
            return Err(invalid_metadata(field, PLENORA_CONTRACT_VERSION_KEY));
        }
        for key in [
            PLENORA_ENCODING_KEY,
            PLENORA_DIMENSIONS_KEY,
            PLENORA_CRS_RESOLUTION_KEY,
            PLENORA_TYPES_DECLARATION_KEY,
        ] {
            if !field.metadata().contains_key(key) {
                return Err(invalid_metadata(field, key));
            }
        }
        if extension.is_some_and(|value| value != GEOARROW_WKB_EXTENSION) {
            return Err(invalid_metadata(field, ARROW_EXTENSION_NAME_KEY));
        }
    }

    if (canonical || is_geoarrow_wkb_field(field))
        && !matches!(
            field.data_type(),
            arrow_schema::DataType::Binary | arrow_schema::DataType::LargeBinary
        )
    {
        return Err(invalid_metadata(field, PLENORA_ENCODING_KEY));
    }
    Ok(())
}

#[must_use]
pub fn with_contract_version(schema: arrow_schema::SchemaRef) -> arrow_schema::SchemaRef {
    let mut metadata = schema.metadata().clone();
    metadata.insert(
        PLENORA_CONTRACT_VERSION_KEY.to_owned(),
        PLENORA_CONTRACT_VERSION.to_owned(),
    );
    Arc::new(arrow_schema::Schema::new_with_metadata(
        schema.fields().clone(),
        metadata,
    ))
}

pub fn validate_contract_version(schema: &arrow_schema::Schema) -> Result<()> {
    match schema.metadata().get(PLENORA_CONTRACT_VERSION_KEY) {
        None => Ok(()),
        Some(version) if version == PLENORA_CONTRACT_VERSION => Ok(()),
        Some(_) => Err(PlenoraIoError::Contract(
            "versione del protocollo metadati non supportata".to_owned(),
        )),
    }
}

fn axis_order_name(order: AxisOrder) -> &'static str {
    match order {
        AxisOrder::LongitudeLatitude => "lon_lat",
        AxisOrder::LatitudeLongitude => "lat_lon",
        AxisOrder::EastingNorthing => "easting_northing",
        AxisOrder::NorthingEasting => "northing_easting",
        AxisOrder::Other => "other",
        AxisOrder::Unknown => "unknown",
    }
}

fn definition_format_name(format: CrsDefinitionFormat) -> &'static str {
    match format {
        CrsDefinitionFormat::Wkt => "wkt",
        CrsDefinitionFormat::Wkt2 => "wkt2",
        CrsDefinitionFormat::Projjson => "projjson",
    }
}

/// Registra nel campo Arrow le parti del contratto che GeoArrow-WKB da solo
/// non esprime. Le chiavi sono namespaced e quindi sopravvivono in Arrow IPC.
pub fn with_geometry_contract_metadata(
    field: &arrow_schema::Field,
    contract: &GeometryColumnContract,
) -> arrow_schema::Field {
    let mut metadata = field.metadata().clone();
    metadata.insert(
        PLENORA_FIELD_ID_KEY.to_owned(),
        contract.field_id.0.to_string(),
    );
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
    let declaration_name = match contract.types_declaration {
        TypesDeclaration::Exact => Some("exact"),
        TypesDeclaration::Mixed => Some("mixed"),
        TypesDeclaration::Unresolved => Some("unresolved"),
        TypesDeclaration::LegacyUndeclared => None,
    };
    if let Some(declaration_name) = declaration_name {
        metadata.insert(
            PLENORA_TYPES_DECLARATION_KEY.to_owned(),
            declaration_name.to_owned(),
        );
        if contract.types_declaration == TypesDeclaration::Unresolved {
            metadata.remove(PLENORA_GEOMETRY_TYPES_KEY);
        } else {
            metadata.insert(
                PLENORA_GEOMETRY_TYPES_KEY.to_owned(),
                contract
                    .geometry_types
                    .iter()
                    .map(|geometry_type| geometry_type.canonical_name())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
    } else {
        metadata.remove(PLENORA_TYPES_DECLARATION_KEY);
        metadata.remove(PLENORA_GEOMETRY_TYPES_KEY);
    }
    for key in [
        PLENORA_CRS_ID_KEY,
        PLENORA_CRS_DEFINITION_KEY,
        PLENORA_CRS_DEFINITION_FORMAT_KEY,
        PLENORA_AXIS_ORDER_KEY,
    ] {
        metadata.remove(key);
    }
    match &contract.crs {
        CrsResolution::Resolved(crs) => {
            metadata.insert(PLENORA_CRS_RESOLUTION_KEY.to_owned(), "resolved".to_owned());
            if let Some(id) = &crs.id {
                metadata.insert(PLENORA_CRS_ID_KEY.to_owned(), id.clone());
            }
            if let Some(definition) = &crs.definition {
                metadata.insert(PLENORA_CRS_DEFINITION_KEY.to_owned(), definition.clone());
                let format = match crs.definition_format {
                    Some(format) => Some(format),
                    None => {
                        ResolvedCrs::new(None, crs.kind, Some(definition.clone())).definition_format
                    }
                };
                if let Some(format) = format {
                    metadata.insert(
                        PLENORA_CRS_DEFINITION_FORMAT_KEY.to_owned(),
                        definition_format_name(format).to_owned(),
                    );
                }
            }
            metadata.insert(
                PLENORA_AXIS_ORDER_KEY.to_owned(),
                axis_order_name(crs.axis_order).to_owned(),
            );
        }
        CrsResolution::DeclaredButUnresolved(raw) => {
            metadata.insert(
                PLENORA_CRS_RESOLUTION_KEY.to_owned(),
                "declared_unresolved".to_owned(),
            );
            if let Some(id) = &raw.authority_hint {
                metadata.insert(PLENORA_CRS_ID_KEY.to_owned(), id.clone());
            }
            if let Some(definition) = &raw.definition {
                metadata.insert(PLENORA_CRS_DEFINITION_KEY.to_owned(), definition.clone());
                let format = match raw.definition_format {
                    Some(format) => format,
                    None => definition_format(definition),
                };
                metadata.insert(
                    PLENORA_CRS_DEFINITION_FORMAT_KEY.to_owned(),
                    definition_format_name(format).to_owned(),
                );
            }
            if raw.authority_hint.is_some() || raw.definition.is_some() {
                metadata.insert(
                    PLENORA_AXIS_ORDER_KEY.to_owned(),
                    axis_order_name(raw.axis_order).to_owned(),
                );
            }
        }
        CrsResolution::Missing => {
            metadata.insert(PLENORA_CRS_RESOLUTION_KEY.to_owned(), "missing".to_owned());
        }
    }
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

    if let Some(value) = metadata.get(PLENORA_FIELD_ID_KEY) {
        parsed.field_id = crate::contract::FieldId(
            value
                .parse()
                .map_err(|_| invalid_metadata(field, PLENORA_FIELD_ID_KEY))?,
        );
    }
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
                if geometry_types
                    .last()
                    .is_some_and(|previous| *previous >= geometry_type)
                {
                    return Err(invalid_metadata(field, PLENORA_GEOMETRY_TYPES_KEY));
                }
                geometry_types.push(geometry_type);
            }
        }
        parsed.geometry_types = geometry_types;
    }
    if let Some(value) = metadata.get(PLENORA_TYPES_DECLARATION_KEY) {
        parsed.types_declaration = match value.as_str() {
            "exact" => TypesDeclaration::Exact,
            "mixed" => TypesDeclaration::Mixed,
            "unresolved" => TypesDeclaration::Unresolved,
            _ => return Err(invalid_metadata(field, PLENORA_TYPES_DECLARATION_KEY)),
        };
    } else {
        parsed.types_declaration = TypesDeclaration::LegacyUndeclared;
    }
    match parsed.types_declaration {
        TypesDeclaration::Exact if parsed.geometry_types.is_empty() => {
            return Err(invalid_metadata(field, PLENORA_GEOMETRY_TYPES_KEY));
        }
        TypesDeclaration::Unresolved if !parsed.geometry_types.is_empty() => {
            return Err(invalid_metadata(field, PLENORA_TYPES_DECLARATION_KEY));
        }
        TypesDeclaration::Exact
        | TypesDeclaration::Mixed
        | TypesDeclaration::Unresolved
        | TypesDeclaration::LegacyUndeclared => {}
    }
    let resolution = metadata.get(PLENORA_CRS_RESOLUTION_KEY);
    if let Some(resolution) = resolution {
        let id = metadata.get(PLENORA_CRS_ID_KEY).cloned();
        let definition = metadata.get(PLENORA_CRS_DEFINITION_KEY).cloned();
        let definition_format = metadata
            .get(PLENORA_CRS_DEFINITION_FORMAT_KEY)
            .map(|value| match value.as_str() {
                "wkt" => Ok(CrsDefinitionFormat::Wkt),
                "wkt2" => Ok(CrsDefinitionFormat::Wkt2),
                "projjson" => Ok(CrsDefinitionFormat::Projjson),
                _ => Err(invalid_metadata(field, PLENORA_CRS_DEFINITION_FORMAT_KEY)),
            })
            .transpose()?;
        let axis_order = metadata
            .get(PLENORA_AXIS_ORDER_KEY)
            .map(|value| match value.as_str() {
                "lon_lat" => Ok(AxisOrder::LongitudeLatitude),
                "lat_lon" => Ok(AxisOrder::LatitudeLongitude),
                "easting_northing" => Ok(AxisOrder::EastingNorthing),
                "northing_easting" => Ok(AxisOrder::NorthingEasting),
                "other" => Ok(AxisOrder::Other),
                "unknown" => Ok(AxisOrder::Unknown),
                _ => Err(invalid_metadata(field, PLENORA_AXIS_ORDER_KEY)),
            })
            .transpose()?;
        if definition.is_some() != definition_format.is_some() {
            return Err(invalid_metadata(field, PLENORA_CRS_DEFINITION_FORMAT_KEY));
        }
        parsed.crs = match resolution.as_str() {
            "resolved" => {
                if id.is_none() && definition.is_none() {
                    return Err(invalid_metadata(field, PLENORA_CRS_RESOLUTION_KEY));
                }
                let axis_order =
                    axis_order.ok_or_else(|| invalid_metadata(field, PLENORA_AXIS_ORDER_KEY))?;
                let kind = match axis_order {
                    AxisOrder::LongitudeLatitude | AxisOrder::LatitudeLongitude => {
                        crate::crs::CrsKind::Geographic
                    }
                    AxisOrder::EastingNorthing | AxisOrder::NorthingEasting => {
                        crate::crs::CrsKind::Projected
                    }
                    AxisOrder::Other | AxisOrder::Unknown => crate::crs::CrsKind::Unknown,
                };
                let mut crs = ResolvedCrs::new(id, kind, definition);
                crs.definition_format = definition_format;
                crs.axis_order = axis_order;
                CrsResolution::Resolved(crs)
            }
            "declared_unresolved" => {
                let has_structured_representation = id.is_some() || definition.is_some();
                if !has_structured_representation && parsed.srid.is_none() {
                    return Err(invalid_metadata(field, PLENORA_CRS_RESOLUTION_KEY));
                }
                if !has_structured_representation && axis_order.is_some() {
                    return Err(invalid_metadata(field, PLENORA_AXIS_ORDER_KEY));
                }
                let axis_order = if has_structured_representation {
                    axis_order.ok_or_else(|| invalid_metadata(field, PLENORA_AXIS_ORDER_KEY))?
                } else {
                    AxisOrder::Unknown
                };
                CrsResolution::DeclaredButUnresolved(RawCrs {
                    definition,
                    authority_hint: id,
                    definition_format,
                    axis_order,
                })
            }
            "missing"
                if id.is_none()
                    && definition.is_none()
                    && axis_order.is_none()
                    && parsed.srid.is_none() =>
            {
                CrsResolution::Missing
            }
            _ => return Err(invalid_metadata(field, PLENORA_CRS_RESOLUTION_KEY)),
        };
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
            Err(error) if error.category == crate::ErrorCategory::InvalidPlan
        ));
        assert_eq!(geometry.encoding, original.encoding);
        assert_eq!(geometry.precision, original.precision);
    }

    #[test]
    fn invalid_srid_and_geometry_types_are_rejected() {
        for field in [
            field_with(PLENORA_FIELD_ID_KEY, "not-a-u32"),
            field_with(PLENORA_SRID_KEY, "not-an-i32"),
            field_with(PLENORA_GEOMETRY_TYPES_KEY, "point,futuregeometry"),
            field_with(PLENORA_GEOMETRY_TYPES_KEY, "point,point"),
            field_with(PLENORA_GEOMETRY_TYPES_KEY, "polygon,point"),
            field_with(PLENORA_GEOMETRY_TYPES_KEY, "line_string"),
            field_with(PLENORA_GEOMETRY_TYPES_KEY, "LineString"),
        ] {
            assert!(matches!(
                read_geometry_contract_metadata(&field, &mut contract()),
                Err(error) if error.category == crate::ErrorCategory::InvalidPlan
            ));
        }
    }

    #[test]
    fn geometry_type_metadata_uses_canonical_names_without_separators() {
        let mut geometry = contract();
        geometry.types_declaration = TypesDeclaration::Exact;
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

    #[test]
    fn emits_and_reads_complete_crs_and_type_contract() {
        let mut geometry = GeometryColumnContract::wkb_xy(
            FieldId(17),
            "geometry",
            ResolvedCrs::new(
                Some("EPSG:4326".to_owned()),
                crate::crs::CrsKind::Geographic,
                Some("GEOGCRS[\"WGS 84\"]".to_owned()),
            ),
            true,
        );
        geometry.geometry_types = vec![GeometryType::Point];
        geometry.types_declaration = TypesDeclaration::Exact;
        let field = with_geometry_contract_metadata(
            &Field::new("geometry", DataType::Binary, true),
            &geometry,
        );
        let metadata = field.metadata();
        assert_eq!(
            metadata.get(PLENORA_FIELD_ID_KEY).map(String::as_str),
            Some("17")
        );
        assert_eq!(
            metadata.get(PLENORA_CRS_ID_KEY).map(String::as_str),
            Some("EPSG:4326")
        );
        assert_eq!(
            metadata.get(PLENORA_CRS_RESOLUTION_KEY).map(String::as_str),
            Some("resolved")
        );
        assert_eq!(
            metadata
                .get(PLENORA_CRS_DEFINITION_FORMAT_KEY)
                .map(String::as_str),
            Some("wkt2")
        );
        assert_eq!(
            metadata.get(PLENORA_AXIS_ORDER_KEY).map(String::as_str),
            Some("lat_lon")
        );
        assert_eq!(
            metadata
                .get(PLENORA_TYPES_DECLARATION_KEY)
                .map(String::as_str),
            Some("exact")
        );

        let mut decoded = contract();
        read_geometry_contract_metadata(&field, &mut decoded).unwrap();
        assert_eq!(decoded.field_id, FieldId(17));
        assert_eq!(decoded.crs, geometry.crs);
        assert_eq!(decoded.types_declaration, TypesDeclaration::Exact);
    }

    #[test]
    fn schema_contract_version_is_emitted_and_future_versions_fail_closed() {
        let schema = Arc::new(arrow_schema::Schema::new(vec![Field::new(
            "geometry",
            DataType::Binary,
            true,
        )]));
        let schema = with_contract_version(schema);
        assert_eq!(
            schema
                .metadata()
                .get(PLENORA_CONTRACT_VERSION_KEY)
                .map(String::as_str),
            Some("1")
        );
        validate_contract_version(schema.as_ref()).unwrap();

        let future = arrow_schema::Schema::new_with_metadata(
            Vec::<Field>::new(),
            HashMap::from([(PLENORA_CONTRACT_VERSION_KEY.to_owned(), "2".to_owned())]),
        );
        assert!(validate_contract_version(&future).is_err());
    }

    #[test]
    fn legacy_missing_type_declaration_is_preserved_as_absence() {
        let field =
            Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([(
                ARROW_EXTENSION_NAME_KEY.to_owned(),
                GEOARROW_WKB_EXTENSION.to_owned(),
            )]));
        let schema = Arc::new(arrow_schema::Schema::new(vec![field.clone()]));
        let mut geometry = contract();

        read_geometry_contract_metadata(&field, &mut geometry).unwrap();
        assert_eq!(
            geometry.types_declaration,
            TypesDeclaration::LegacyUndeclared
        );

        let data_contract = crate::contract::DataContract::new(schema.clone(), Some(geometry));
        assert!(data_contract
            .schema
            .metadata()
            .get(PLENORA_CONTRACT_VERSION_KEY)
            .is_none());
        assert_eq!(data_contract.schema, schema);
    }

    #[test]
    fn unresolved_crs_preserves_definition_format_and_axis_order() {
        let mut geometry = contract();
        geometry.crs = CrsResolution::DeclaredButUnresolved(RawCrs {
            definition: Some("GEOGCRS[\"unresolved\"]".to_owned()),
            authority_hint: Some("EPSG:4326".to_owned()),
            definition_format: Some(CrsDefinitionFormat::Wkt2),
            axis_order: AxisOrder::LatitudeLongitude,
        });
        let field = with_geometry_contract_metadata(
            &Field::new("geometry", DataType::Binary, true),
            &geometry,
        );
        let mut decoded = contract();
        read_geometry_contract_metadata(&field, &mut decoded).unwrap();
        assert_eq!(decoded.crs, geometry.crs);
    }

    #[test]
    fn unresolved_crs_roundtrips_with_authority_only() {
        let mut geometry = contract();
        geometry.crs = CrsResolution::DeclaredButUnresolved(RawCrs::from_authority_hint(
            "EPSG:99999".to_owned(),
        ));
        let field = with_geometry_contract_metadata(
            &Field::new("geometry", DataType::Binary, true),
            &geometry,
        );

        assert_eq!(
            field.metadata().get(PLENORA_CRS_ID_KEY).map(String::as_str),
            Some("EPSG:99999")
        );
        assert!(!field.metadata().contains_key(PLENORA_CRS_DEFINITION_KEY));
        assert!(!field
            .metadata()
            .contains_key(PLENORA_CRS_DEFINITION_FORMAT_KEY));

        let mut decoded = contract();
        read_geometry_contract_metadata(&field, &mut decoded).unwrap();
        assert_eq!(decoded.crs, geometry.crs);
    }

    #[test]
    fn unresolved_crs_roundtrips_with_srid_only_without_synthesis() {
        let field = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([
            (
                PLENORA_CRS_RESOLUTION_KEY.to_owned(),
                "declared_unresolved".to_owned(),
            ),
            (PLENORA_SRID_KEY.to_owned(), "4326".to_owned()),
        ]));
        let mut geometry = contract();

        read_geometry_contract_metadata(&field, &mut geometry).unwrap();

        assert_eq!(geometry.srid, Some(4326));
        let raw = geometry.crs.raw().unwrap();
        assert_eq!(raw.authority_hint, None);
        assert_eq!(raw.definition, None);
        assert_eq!(raw.definition_format, None);
        assert_eq!(raw.axis_order, AxisOrder::Unknown);

        let emitted = with_geometry_contract_metadata(
            &Field::new("geometry", DataType::Binary, true),
            &geometry,
        );
        assert_eq!(
            emitted.metadata().get(PLENORA_SRID_KEY).map(String::as_str),
            Some("4326")
        );
        for key in [
            PLENORA_CRS_ID_KEY,
            PLENORA_CRS_DEFINITION_KEY,
            PLENORA_CRS_DEFINITION_FORMAT_KEY,
            PLENORA_AXIS_ORDER_KEY,
        ] {
            assert!(
                !emitted.metadata().contains_key(key),
                "chiave sintetizzata: {key}"
            );
        }
    }

    #[test]
    fn resolved_crs_with_srid_only_is_rejected() {
        let field = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([
            (PLENORA_CRS_RESOLUTION_KEY.to_owned(), "resolved".to_owned()),
            (PLENORA_SRID_KEY.to_owned(), "4326".to_owned()),
        ]));

        assert!(read_geometry_contract_metadata(&field, &mut contract()).is_err());
    }

    #[test]
    fn unresolved_crs_with_structured_representation_requires_axis_order() {
        for key in [PLENORA_CRS_ID_KEY, PLENORA_CRS_DEFINITION_KEY] {
            let mut metadata = HashMap::from([
                (
                    PLENORA_CRS_RESOLUTION_KEY.to_owned(),
                    "declared_unresolved".to_owned(),
                ),
                (key.to_owned(), "EPSG:4326".to_owned()),
            ]);
            if key == PLENORA_CRS_DEFINITION_KEY {
                metadata.insert(
                    PLENORA_CRS_DEFINITION_FORMAT_KEY.to_owned(),
                    "wkt".to_owned(),
                );
            }
            let field = Field::new("geometry", DataType::Binary, true).with_metadata(metadata);

            assert!(read_geometry_contract_metadata(&field, &mut contract()).is_err());
        }
    }

    #[test]
    fn axis_order_without_structured_crs_is_rejected() {
        for (resolution, srid) in [("declared_unresolved", Some("4326")), ("missing", None)] {
            let mut metadata = HashMap::from([
                (PLENORA_CRS_RESOLUTION_KEY.to_owned(), resolution.to_owned()),
                (PLENORA_AXIS_ORDER_KEY.to_owned(), "lon_lat".to_owned()),
            ]);
            if let Some(srid) = srid {
                metadata.insert(PLENORA_SRID_KEY.to_owned(), srid.to_owned());
            }
            let field = Field::new("geometry", DataType::Binary, true).with_metadata(metadata);

            assert!(
                read_geometry_contract_metadata(&field, &mut contract()).is_err(),
                "`{resolution}` senza crs_id/definition ha accettato axis_order"
            );
        }
    }

    #[test]
    fn unresolved_crs_without_any_declaration_is_rejected() {
        let field = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([
            (
                PLENORA_CRS_RESOLUTION_KEY.to_owned(),
                "declared_unresolved".to_owned(),
            ),
            (PLENORA_AXIS_ORDER_KEY.to_owned(), "unknown".to_owned()),
        ]));

        assert!(read_geometry_contract_metadata(&field, &mut contract()).is_err());
    }
}
