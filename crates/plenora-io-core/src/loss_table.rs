//! Regole di trasporto dello schema, condivise dal catalogo e dal piano di perdita.

use crate::property_plan::Rule;
use arrow_schema::DataType;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportDirection {
    Read,
    Write,
}

/// Qualification properties, with GPKG write/read rules and opaque fallback.
///
/// Reconstructed declares catalog/native reconstruction, not value inequality.
/// Only logical identity and contract version promise metadata preservation.
pub const PROPERTIES: &[(&str, &str, Rule, Rule)] = &[
    (
        "contract_version",
        "plenora.contract.version",
        Rule::Preserved,
        Rule::Preserved,
    ),
    (
        "field_id",
        "plenora.field_id",
        Rule::Preserved,
        Rule::Preserved,
    ),
    (
        "types_declaration",
        "plenora.geometry.types_declaration",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "geometry_types",
        "plenora.geometry.types",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "dimensions",
        "plenora.geometry.dimensions",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "encoding",
        "plenora.geometry.encoding",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "geometry_or_geography",
        "plenora.geometry.spatial_semantics",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "precision",
        "plenora.geometry.precision",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "srid",
        "plenora.geometry.srid",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "crs_id",
        "plenora.geometry.crs_id",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "crs_resolution",
        "plenora.geometry.crs_resolution",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "crs_definition",
        "plenora.geometry.crs_definition",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "crs_definition_format",
        "plenora.geometry.crs_definition_format",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "axis_order",
        "plenora.geometry.axis_order",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "native_metadata",
        "plenora.geometry.native.",
        Rule::Removed,
        Rule::Preserved,
    ),
    ("", "gpkg.declared_type", Rule::Preserved, Rule::Preserved),
    ("", "gpkg.primary_key", Rule::Preserved, Rule::Preserved),
    (
        "",
        "ARROW:extension:name",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "",
        "ARROW:extension:metadata",
        Rule::Removed,
        Rule::Preserved,
    ),
    (
        "",
        "plenora.primary_geometry",
        Rule::Removed,
        Rule::Preserved,
    ),
    ("", "gpkg.schema_normalized", Rule::Removed, Rule::Preserved),
    ("", "crs", Rule::Removed, Rule::Preserved),
    ("", "geo", Rule::Removed, Rule::Preserved),
    ("", "", Rule::Removed, Rule::Rejected),
];

use plenora_io_model::contract::CoordinateDimensions;

use crate::descriptor::{ArrowTypeClass, ALL_ARROW_TYPES, SCALAR_TYPES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataTransport {
    Arrow,
    GeoPackage,
    Native,
}

#[derive(Clone, Copy, Debug)]
pub struct DriverRules {
    pub driver: &'static str,
    pub allowed_types: &'static [ArrowTypeClass],
    pub stable_types: &'static [DataType],
    pub metadata: MetadataTransport,
    pub dimensions: &'static [CoordinateDimensions],
}

const TEXT_SCALARS: &[ArrowTypeClass] = &[
    ArrowTypeClass::Boolean,
    ArrowTypeClass::SignedInteger,
    ArrowTypeClass::UnsignedInteger,
    ArrowTypeClass::Floating,
    ArrowTypeClass::Utf8,
    ArrowTypeClass::Temporal,
];
const SQL_SCALARS: &[ArrowTypeClass] = &[
    ArrowTypeClass::Boolean,
    ArrowTypeClass::SignedInteger,
    ArrowTypeClass::UnsignedInteger,
    ArrowTypeClass::Floating,
    ArrowTypeClass::Utf8,
    ArrowTypeClass::Binary,
];
const GDB_SCALARS: &[ArrowTypeClass] = &[
    ArrowTypeClass::SignedInteger,
    ArrowTypeClass::Floating,
    ArrowTypeClass::Utf8,
];

pub const CSV: DriverRules = DriverRules {
    driver: "csv",
    dimensions: ALL_DIMENSIONS,
    allowed_types: TEXT_SCALARS,
    stable_types: &[DataType::Utf8],
    metadata: MetadataTransport::Native,
};
pub const SHP: DriverRules = DriverRules {
    driver: "shp",
    dimensions: ALL_DIMENSIONS,
    allowed_types: SCALAR_TYPES,
    stable_types: &[
        DataType::Int64,
        DataType::Float64,
        DataType::Boolean,
        DataType::Utf8,
    ],
    metadata: MetadataTransport::Native,
};
pub const GPKG: DriverRules = DriverRules {
    driver: "gpkg",
    dimensions: ALL_DIMENSIONS,
    allowed_types: SQL_SCALARS,
    stable_types: &[
        DataType::Int64,
        DataType::Float64,
        DataType::Utf8,
        DataType::Binary,
    ],
    metadata: MetadataTransport::GeoPackage,
};
pub const FILEGDB: DriverRules = DriverRules {
    driver: "filegdb",
    dimensions: GDB_DIMENSIONS,
    allowed_types: GDB_SCALARS,
    stable_types: &[
        DataType::Int32,
        DataType::Int64,
        DataType::Float64,
        DataType::Utf8,
    ],
    metadata: MetadataTransport::Native,
};
pub const GEOJSON: DriverRules = DriverRules {
    driver: "geojson",
    dimensions: XYZ_DIMENSIONS,
    allowed_types: TEXT_SCALARS,
    stable_types: &[
        DataType::Int64,
        DataType::Float64,
        DataType::Boolean,
        DataType::Utf8,
    ],
    metadata: MetadataTransport::Native,
};
pub const KML: DriverRules = DriverRules {
    driver: "kml",
    dimensions: XYZ_DIMENSIONS,
    allowed_types: TEXT_SCALARS,
    stable_types: &[DataType::Utf8],
    metadata: MetadataTransport::Native,
};
pub const DXF: DriverRules = DriverRules {
    driver: "dxf",
    dimensions: XYZ_DIMENSIONS,
    allowed_types: SCALAR_TYPES,
    stable_types: &[],
    metadata: MetadataTransport::Native,
};
pub const XLS: DriverRules = DriverRules {
    driver: "xls",
    dimensions: ALL_DIMENSIONS,
    allowed_types: TEXT_SCALARS,
    stable_types: &[DataType::Float64, DataType::Boolean, DataType::Utf8],
    metadata: MetadataTransport::Native,
};
pub const GEOPARQUET: DriverRules = DriverRules {
    driver: "geoparquet",
    dimensions: XYZ_DIMENSIONS,
    allowed_types: ALL_ARROW_TYPES,
    stable_types: &[],
    metadata: MetadataTransport::Arrow,
};
pub const IPC: DriverRules = DriverRules {
    driver: "ipc",
    dimensions: ALL_DIMENSIONS,
    allowed_types: ALL_ARROW_TYPES,
    stable_types: &[],
    metadata: MetadataTransport::Arrow,
};

pub const DRIVERS: &[DriverRules] = &[
    CSV, SHP, GPKG, FILEGDB, GEOJSON, KML, DXF, XLS, GEOPARQUET, IPC,
];

const XYZ_DIMENSIONS: &[CoordinateDimensions] = &[
    CoordinateDimensions::Xy,
    CoordinateDimensions::Xyz,
    CoordinateDimensions::Unknown,
];
const GDB_DIMENSIONS: &[CoordinateDimensions] = &[
    CoordinateDimensions::Xy,
    CoordinateDimensions::Xyz,
    CoordinateDimensions::Xym,
    CoordinateDimensions::Xyzm,
];
const ALL_DIMENSIONS: &[CoordinateDimensions] = &[
    CoordinateDimensions::Xy,
    CoordinateDimensions::Xyz,
    CoordinateDimensions::Xym,
    CoordinateDimensions::Xyzm,
    CoordinateDimensions::Unknown,
];

#[must_use]
pub fn rules(driver: &str) -> Option<&'static DriverRules> {
    DRIVERS.iter().find(|row| row.driver == driver)
}

#[must_use]
pub fn normalizes_type(driver: &str, data_type: &DataType) -> bool {
    rules(driver).is_some_and(|row| {
        row.metadata != MetadataTransport::Arrow && !row.stable_types.contains(data_type)
    })
}

/// # Errors
/// Rejects native missing measures when their interpretation cannot travel.
pub fn validate_native_measures(
    driver: &'static str,
    geometries: &[plenora_io_model::contract::GeometryColumnContract],
) -> plenora_io_model::Result<()> {
    if !matches!(driver, "ipc" | "shp" | "geoparquet")
        && geometries
            .iter()
            .any(|geometry| geometry.native_metadata.contains_key("shp.measure_no_data"))
    {
        return Err(plenora_io_model::PlenoraIoError::capability_redatta(
            driver,
            None,
            plenora_io_model::CapabilityReason::TypeNotRepresentable,
            &plenora_io_model::PublicMessage::Curated(
                "misura no-data nativa non rappresentabile nella destinazione",
            ),
        ));
    }
    Ok(())
}

#[must_use]
pub fn metadata_rule(
    driver: &str,
    direction: TransportDirection,
    key: &str,
) -> crate::property_plan::Rule {
    match rules(driver).map(|row| row.metadata) {
        Some(MetadataTransport::GeoPackage) => PROPERTIES
            .iter()
            .find(|(_, pattern, _, _)| {
                pattern.is_empty()
                    || *pattern == key
                    || (pattern.ends_with('.') && key.starts_with(pattern))
            })
            .map_or(Rule::Rejected, |entry| match direction {
                TransportDirection::Read => entry.3,
                TransportDirection::Write => entry.2,
            }),
        Some(MetadataTransport::Native) if direction == TransportDirection::Write => {
            Rule::ArrowMetadataRemoved
        }
        Some(MetadataTransport::Native | MetadataTransport::Arrow) | None => Rule::Preserved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn missing_measure_cannot_escape_without_its_native_meaning() {
        let mut geometry = plenora_io_model::contract::GeometryColumnContract::wkb_xy(
            plenora_io_model::contract::FieldId(0),
            "geometry",
            plenora_io_model::crs::ResolvedCrs::wgs84(),
            true,
        );
        geometry.dimensions = CoordinateDimensions::Xym;
        geometry
            .native_metadata
            .insert("shp.measure_no_data".to_owned(), "-1e39".to_owned());
        for driver in ["csv", "xls", "geojson", "kml", "dxf", "gpkg", "filegdb"] {
            let error =
                validate_native_measures(driver, std::slice::from_ref(&geometry)).unwrap_err();
            assert_eq!(error.category, plenora_io_model::ErrorCategory::Unsupported);
            assert!(!error.message.contains("-1e39"));
        }
        for driver in ["ipc", "shp", "geoparquet"] {
            validate_native_measures(driver, std::slice::from_ref(&geometry)).unwrap();
        }
        geometry.native_metadata.clear();
        validate_native_measures("csv", &[geometry]).unwrap();
    }

    #[test]
    fn opaque_schema_and_field_metadata_have_counts_without_payload() {
        let schema = arrow_schema::Schema::new_with_metadata(
            vec![
                arrow_schema::Field::new("secret_field", DataType::Utf8, true).with_metadata(
                    HashMap::from([("secret_key".into(), "secret_value".into())]),
                ),
            ],
            HashMap::from([("secret_schema_key".into(), "secret_schema_value".into())]),
        );
        for driver in ["csv", "xls", "geojson", "kml", "dxf", "shp", "filegdb"] {
            let plan = crate::property_plan::PropertyPlan::new(&schema, driver, None);
            plan.validate_schema(driver, &schema).unwrap();
            let report = plan.report(3, crate::property_plan::Phase::Write).unwrap();
            assert_eq!(report.counts.get("arrow_metadata_not_preserved"), Some(&2));
            let json =
                serde_json::to_string(&report.esempi_canonici().collect::<Vec<_>>()).unwrap();
            assert!(!json.contains("secret"));
            assert!(json.contains("key_index=0"));
        }
        let plan = crate::property_plan::PropertyPlan::new(&schema, "ipc", None);
        assert!(plan
            .report(3, crate::property_plan::Phase::Write)
            .unwrap()
            .counts
            .is_empty());
    }
}
