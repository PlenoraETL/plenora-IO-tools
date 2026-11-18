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

use crate::descriptor::{ArrowTypeClass, ALL_ARROW_TYPES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataTransport {
    Arrow,
    GeoPackage,
    Native,
}

#[derive(Clone, Copy, Debug)]
pub struct DriverRules {
    pub driver: &'static str,
    pub qualified_native: Option<NativeDriver>,
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

pub const CSV: DriverRules = DriverRules {
    driver: "csv",
    qualified_native: None,
    dimensions: ALL_DIMENSIONS,
    allowed_types: TEXT_SCALARS,
    stable_types: &[DataType::Utf8],
    metadata: MetadataTransport::Native,
};
pub const SHP: DriverRules = DriverRules {
    driver: "shp",
    qualified_native: Some(NativeDriver::Shp),
    dimensions: ALL_DIMENSIONS,
    allowed_types: &SHP_NATIVE_CLASSES,
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
    qualified_native: None,
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
    qualified_native: Some(NativeDriver::FileGdb),
    dimensions: GDB_DIMENSIONS,
    allowed_types: &FILEGDB_NATIVE_CLASSES,
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
    qualified_native: Some(NativeDriver::GeoJson),
    dimensions: XYZ_DIMENSIONS,
    allowed_types: &GEOJSON_NATIVE_CLASSES,
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
    qualified_native: Some(NativeDriver::Kml),
    dimensions: XYZ_DIMENSIONS,
    allowed_types: &KML_NATIVE_CLASSES,
    stable_types: &[DataType::Utf8],
    metadata: MetadataTransport::Native,
};
pub const DXF: DriverRules = DriverRules {
    driver: "dxf",
    qualified_native: Some(NativeDriver::Dxf),
    dimensions: XYZ_DIMENSIONS,
    allowed_types: &DXF_NATIVE_CLASSES,
    stable_types: &[],
    metadata: MetadataTransport::Native,
};
pub const XLS: DriverRules = DriverRules {
    driver: "xls",
    qualified_native: None,
    dimensions: ALL_DIMENSIONS,
    allowed_types: TEXT_SCALARS,
    stable_types: &[DataType::Float64, DataType::Boolean, DataType::Utf8],
    metadata: MetadataTransport::Native,
};
pub const GEOPARQUET: DriverRules = DriverRules {
    driver: "geoparquet",
    qualified_native: None,
    dimensions: XYZ_DIMENSIONS,
    allowed_types: ALL_ARROW_TYPES,
    stable_types: &[],
    metadata: MetadataTransport::Arrow,
};
pub const IPC: DriverRules = DriverRules {
    driver: "ipc",
    qualified_native: None,
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

/// Native observations use the same rules as schema transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeChange {
    DxfAttribute,
    NativeHierarchy,
    NativeStyles,
    CoordinatePrecision,
    RingOrientation,
    ArrowMetadata,
    ArrowCoercion,
    FieldNames,
    CrsId,
    CrsDefinition,
    NotNull,
    FieldOrder,
    FeatureId,
    Bbox,
    ForeignMembers,
    Inference,
    NumberPrecision,
    StructuredValue,
    MissingValue,
    Dimensions,
    ExtendedData,
    FeatureMetadata,
    LayerMetadata,
    Styles,
    Hierarchy,
    ExternalReference,
    GeometryType,
    AltitudeMode,
}
const NATIVE_RINGORIENTATION: &str = "native_ring_orientation_changed";
const NATIVE_COORDINATEPRECISION: &str = "native_coordinate_precision";
const NATIVE_NATIVESTYLES: &str = "native_styles_not_preserved";
const NATIVE_NATIVEHIERARCHY: &str = "native_hierarchy_flattened";
const NATIVE_DXFATTRIBUTE: &str = "dxf_attribute_not_represented";
const NATIVE_ARROW_COERCION: &str = "coercion tipo attributo";
const NATIVE_FIELD_NAMES: &str = "native_fields_reconstructed";
const NATIVE_ARROW_METADATA: &str = "arrow_metadata_not_preserved";
const NATIVE_CRS_ID: &str = "crs_id_not_preserved_derived";
const NATIVE_CRS_DEFINITION: &str = "crs_definition_not_preserved_absent";
const NATIVE_NOTNULL: &str = "native_not_null_not_preserved";
const NATIVE_FIELDORDER: &str = "native_field_order_changed";
const NATIVE_FEATUREID: &str = "native_feature_id_not_preserved";
const NATIVE_BBOX: &str = "native_bbox_not_preserved";
const NATIVE_FOREIGNMEMBERS: &str = "native_foreign_members_not_preserved";
const NATIVE_INFERENCE: &str = "native_type_inference";
const NATIVE_NUMBERPRECISION: &str = "native_number_precision";
const NATIVE_STRUCTUREDVALUE: &str = "native_structured_value_rewritten";
const NATIVE_MISSINGVALUE: &str = "native_missing_as_null";
const NATIVE_DIMENSIONS: &str = "native_dimensions_not_declared";
const NATIVE_EXTENDEDDATA: &str = "kml_extended_data_not_preserved";
const NATIVE_FEATUREMETADATA: &str = "kml_feature_metadata_not_preserved";
const NATIVE_LAYERMETADATA: &str = "native_layer_metadata_not_preserved";
const NATIVE_STYLES: &str = "kml_styles_not_preserved";
const NATIVE_HIERARCHY: &str = "kml_hierarchy_flattened";
const NATIVE_EXTERNALREFERENCE: &str = "native_external_reference_not_followed";
const NATIVE_GEOMETRYTYPE: &str = "native_geometry_type_changed";
const NATIVE_ALTITUDEMODE: &str = "kml_altitude_mode_not_preserved";
impl NativeChange {
    #[must_use]
    pub const fn category(self) -> &'static str {
        match self {
            Self::RingOrientation => NATIVE_RINGORIENTATION,
            Self::CoordinatePrecision => NATIVE_COORDINATEPRECISION,
            Self::NativeStyles => NATIVE_NATIVESTYLES,
            Self::NativeHierarchy => NATIVE_NATIVEHIERARCHY,
            Self::DxfAttribute => NATIVE_DXFATTRIBUTE,
            Self::ArrowCoercion => NATIVE_ARROW_COERCION,
            Self::FieldNames => NATIVE_FIELD_NAMES,
            Self::ArrowMetadata => NATIVE_ARROW_METADATA,
            Self::CrsId => NATIVE_CRS_ID,
            Self::CrsDefinition => NATIVE_CRS_DEFINITION,
            Self::NotNull => NATIVE_NOTNULL,
            Self::FieldOrder => NATIVE_FIELDORDER,
            Self::FeatureId => NATIVE_FEATUREID,
            Self::Bbox => NATIVE_BBOX,
            Self::ForeignMembers => NATIVE_FOREIGNMEMBERS,
            Self::Inference => NATIVE_INFERENCE,
            Self::NumberPrecision => NATIVE_NUMBERPRECISION,
            Self::StructuredValue => NATIVE_STRUCTUREDVALUE,
            Self::MissingValue => NATIVE_MISSINGVALUE,
            Self::Dimensions => NATIVE_DIMENSIONS,
            Self::ExtendedData => NATIVE_EXTENDEDDATA,
            Self::FeatureMetadata => NATIVE_FEATUREMETADATA,
            Self::LayerMetadata => NATIVE_LAYERMETADATA,
            Self::Styles => NATIVE_STYLES,
            Self::Hierarchy => NATIVE_HIERARCHY,
            Self::ExternalReference => NATIVE_EXTERNALREFERENCE,
            Self::GeometryType => NATIVE_GEOMETRYTYPE,
            Self::AltitudeMode => NATIVE_ALTITUDEMODE,
        }
    }
}

/// Each row requires both directions for every qualified native driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeDriver {
    GeoJson,
    Kml,
    Shp,
    Dxf,
    FileGdb,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeRule {
    Preserved,
    Transformed(NativeChange),
    Rejected,
}

macro_rules! native_elements {
    ($($element:ident => [$gr:expr, $gw:expr, $kr:expr, $kw:expr, $sr:expr, $sw:expr, $dr:expr, $dw:expr, $fr:expr, $fw:expr];)+) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum NativeElement { $($element,)+ }
        pub const NATIVE_ELEMENTS: &[NativeElement] = &[$(NativeElement::$element,)+];
        impl NativeElement {
            #[must_use]
            pub const fn rules(self) -> [NativeRule; 10] {
                match self { $(Self::$element => [$gr, $gw, $kr, $kw, $sr, $sw, $dr, $dw, $fr, $fw],)+ }
            }
        }
    };
}
use NativeRule::{Preserved as P, Rejected as R, Transformed as T};
native_elements! {
    NullGeometry => [P, P, P, P, P, P, R, R, P, P];
    EmptyPoint => [R, R, R, R, P, P, R, R, T(NativeChange::MissingValue), T(NativeChange::MissingValue)];
    SignedZero => [P, P, P, P, P, P, P, T(NativeChange::CoordinatePrecision), P, T(NativeChange::CoordinatePrecision)];
    Subtypes => [R, R, R, R, R, R, R, R, T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata)];
    Aliases => [R, R, R, R, R, R, R, R, T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata)];
    Domains => [R, R, R, R, R, R, R, R, T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata)];
    TextStyle => [R, R, R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R];
    PaperSpace => [R, R, R, R, R, R, T(NativeChange::NativeHierarchy), T(NativeChange::ArrowMetadata), R, R];
    Xdata => [R, R, R, R, R, R, T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), R, R];
    Width => [R, R, R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R];
    Thickness => [R, R, R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R];
    Lineweight => [R, R, R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R];
    Linetype => [R, R, R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R];
    Color => [R, R, R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R];
    Handle => [R, R, R, R, R, R, T(NativeChange::FeatureId), T(NativeChange::ArrowMetadata), R, R];
    DbfDate => [R, R, R, R, T(NativeChange::ArrowCoercion), T(NativeChange::ArrowCoercion), R, R, R, R];
    Utf8 => [P, P, P, P, P, P, P, T(NativeChange::DxfAttribute), P, P];
    Null => [P, P, P, P, P, P, P, T(NativeChange::DxfAttribute), P, P];
    EmptyString => [P, P, P, P, R, R, P, T(NativeChange::DxfAttribute), P, P];
    SignedInteger => [T(NativeChange::Inference), T(NativeChange::ArrowCoercion), R, T(NativeChange::ArrowCoercion), T(NativeChange::Inference), T(NativeChange::ArrowCoercion), R, T(NativeChange::DxfAttribute), P, T(NativeChange::ArrowCoercion)];
    UnsignedInteger => [T(NativeChange::Inference), T(NativeChange::ArrowCoercion), R, T(NativeChange::ArrowCoercion), R, T(NativeChange::ArrowCoercion), R, T(NativeChange::DxfAttribute), R, R];
    Floating => [T(NativeChange::NumberPrecision), T(NativeChange::ArrowCoercion), R, T(NativeChange::ArrowCoercion), P, T(NativeChange::ArrowCoercion), R, T(NativeChange::DxfAttribute), P, T(NativeChange::ArrowCoercion)];
    Boolean => [P, P, R, T(NativeChange::ArrowCoercion), P, P, R, T(NativeChange::DxfAttribute), R, R];
    Binary => [R, R, R, R, R, T(NativeChange::ArrowCoercion), R, T(NativeChange::DxfAttribute), R, R];
    Decimal => [R, R, R, R, R, T(NativeChange::ArrowCoercion), R, T(NativeChange::DxfAttribute), R, R];
    Temporal => [R, T(NativeChange::ArrowCoercion), R, T(NativeChange::ArrowCoercion), T(NativeChange::ArrowCoercion), T(NativeChange::ArrowCoercion), R, T(NativeChange::DxfAttribute), R, R];
    NestedDictionary => [T(NativeChange::StructuredValue), R, R, R, R, R, R, R, R, R];
    FieldNames => [P, T(NativeChange::FieldNames), T(NativeChange::ExtendedData), T(NativeChange::FieldNames), P, T(NativeChange::FieldNames), T(NativeChange::FieldNames), T(NativeChange::FieldNames), P, T(NativeChange::FieldNames)];
    NotNull => [P, T(NativeChange::NotNull), P, T(NativeChange::NotNull), P, T(NativeChange::NotNull), P, T(NativeChange::NotNull), T(NativeChange::NotNull), T(NativeChange::NotNull)];
    FieldOrder => [T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder)];
    SchemaMetadata => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata)];
    FieldMetadata => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata)];
    NativeMetadata => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata)];
    Z => [P, P, T(NativeChange::AltitudeMode), P, P, P, P, P, P, P];
    M => [R, R, R, R, P, P, R, R, P, P];
    Dimensions => [T(NativeChange::Dimensions), T(NativeChange::Dimensions), T(NativeChange::Dimensions), T(NativeChange::Dimensions), P, P, T(NativeChange::Dimensions), T(NativeChange::Dimensions), P, P];
    GeometryType => [P, P, T(NativeChange::GeometryType), T(NativeChange::GeometryType), T(NativeChange::GeometryType), T(NativeChange::GeometryType), T(NativeChange::GeometryType), T(NativeChange::GeometryType), P, P];
    RingOrientation => [P, P, P, P, P, T(NativeChange::RingOrientation), P, P, P, T(NativeChange::RingOrientation)];
    EmptyGeometry => [R, R, R, R, R, R, R, R, T(NativeChange::MissingValue), T(NativeChange::MissingValue)];
    CoordinatePrecision => [T(NativeChange::NumberPrecision), P, T(NativeChange::NumberPrecision), P, P, P, P, T(NativeChange::CoordinatePrecision), P, T(NativeChange::CoordinatePrecision)];
    CrsId => [P, T(NativeChange::CrsId), P, T(NativeChange::CrsId), P, T(NativeChange::CrsId), P, T(NativeChange::CrsId), P, T(NativeChange::CrsId)];
    CrsDefinition => [P, T(NativeChange::CrsDefinition), P, T(NativeChange::CrsDefinition), P, T(NativeChange::CrsDefinition), P, T(NativeChange::CrsDefinition), P, T(NativeChange::CrsDefinition)];
    FeatureId => [T(NativeChange::FeatureId), T(NativeChange::ArrowMetadata), T(NativeChange::FeatureId), T(NativeChange::ArrowMetadata), T(NativeChange::FeatureId), T(NativeChange::ArrowMetadata), T(NativeChange::FeatureId), T(NativeChange::ArrowMetadata), T(NativeChange::FeatureId), T(NativeChange::ArrowMetadata)];
    Bbox => [T(NativeChange::Bbox), T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata), T(NativeChange::Bbox), T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata), T(NativeChange::Bbox), T(NativeChange::ArrowMetadata)];
    ForeignMembers => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata)];
    LayerMetadata => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata)];
    Styles => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::Styles), T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata), T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata)];
    Hierarchy => [R, R, T(NativeChange::Hierarchy), T(NativeChange::ArrowMetadata), R, R, T(NativeChange::NativeHierarchy), T(NativeChange::ArrowMetadata), T(NativeChange::NativeHierarchy), T(NativeChange::ArrowMetadata)];
    Extensions => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata)];
    ExternalReference => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::ExternalReference), T(NativeChange::ArrowMetadata), R, R, R, T(NativeChange::ArrowMetadata), T(NativeChange::ExternalReference), T(NativeChange::ArrowMetadata)];
    ExtendedData => [R, R, T(NativeChange::ExtendedData), T(NativeChange::ExtendedData), R, R, T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), R, R];
    FeatureMetadata => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::FeatureMetadata), T(NativeChange::ArrowMetadata), R, R, T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), R, R];
    AltitudeMode => [R, R, T(NativeChange::AltitudeMode), P, R, R, R, R, R, R];
    Inference => [T(NativeChange::Inference), T(NativeChange::Inference), R, T(NativeChange::Inference), T(NativeChange::Inference), P, R, R, R, R];
    NumberPrecision => [T(NativeChange::NumberPrecision), P, T(NativeChange::NumberPrecision), P, P, P, P, T(NativeChange::CoordinatePrecision), P, T(NativeChange::CoordinatePrecision)];
    StructuredValue => [T(NativeChange::StructuredValue), R, T(NativeChange::ExtendedData), R, R, R, R, R, R, R];
    MissingValue => [T(NativeChange::MissingValue), P, T(NativeChange::MissingValue), P, P, P, P, P, P, P];
}
#[must_use]
pub const fn native_rule(
    driver: NativeDriver,
    direction: TransportDirection,
    element: NativeElement,
) -> NativeRule {
    let index = match (driver, direction) {
        (NativeDriver::GeoJson, TransportDirection::Read) => 0,
        (NativeDriver::GeoJson, TransportDirection::Write) => 1,
        (NativeDriver::Kml, TransportDirection::Read) => 2,
        (NativeDriver::Kml, TransportDirection::Write) => 3,
        (NativeDriver::Shp, TransportDirection::Read) => 4,
        (NativeDriver::Shp, TransportDirection::Write) => 5,
        (NativeDriver::Dxf, TransportDirection::Read) => 6,
        (NativeDriver::Dxf, TransportDirection::Write) => 7,
        (NativeDriver::FileGdb, TransportDirection::Read) => 8,
        (NativeDriver::FileGdb, TransportDirection::Write) => 9,
    };
    element.rules()[index]
}

const fn native_arrow_element(class: ArrowTypeClass) -> NativeElement {
    match class {
        ArrowTypeClass::Boolean => NativeElement::Boolean,
        ArrowTypeClass::SignedInteger => NativeElement::SignedInteger,
        ArrowTypeClass::UnsignedInteger => NativeElement::UnsignedInteger,
        ArrowTypeClass::Floating => NativeElement::Floating,
        ArrowTypeClass::Utf8 => NativeElement::Utf8,
        ArrowTypeClass::Binary => NativeElement::Binary,
        ArrowTypeClass::Temporal => NativeElement::Temporal,
        ArrowTypeClass::Decimal => NativeElement::Decimal,
        ArrowTypeClass::Nested | ArrowTypeClass::Other => NativeElement::NestedDictionary,
    }
}

const fn native_class_count(driver: NativeDriver) -> usize {
    let mut count = 0;
    let mut index = 0;
    while index < ALL_ARROW_TYPES.len() {
        if !matches!(
            native_rule(
                driver,
                TransportDirection::Write,
                native_arrow_element(ALL_ARROW_TYPES[index])
            ),
            NativeRule::Rejected
        ) {
            count += 1;
        }
        index += 1;
    }
    count
}
const fn native_classes<const N: usize>(driver: NativeDriver) -> [ArrowTypeClass; N] {
    let mut classes = [ArrowTypeClass::Other; N];
    let mut count = 0;
    let mut index = 0;
    while index < ALL_ARROW_TYPES.len() {
        if !matches!(
            native_rule(
                driver,
                TransportDirection::Write,
                native_arrow_element(ALL_ARROW_TYPES[index])
            ),
            NativeRule::Rejected
        ) {
            classes[count] = ALL_ARROW_TYPES[index];
            count += 1;
        }
        index += 1;
    }
    classes
}
const GEOJSON_NATIVE_CLASSES: [ArrowTypeClass; native_class_count(NativeDriver::GeoJson)] =
    native_classes(NativeDriver::GeoJson);
const KML_NATIVE_CLASSES: [ArrowTypeClass; native_class_count(NativeDriver::Kml)] =
    native_classes(NativeDriver::Kml);

/// Null constraints are reconstructed by the native readers.
#[must_use]
pub const fn native_nullability(driver: NativeDriver) -> crate::NullabilitySupport {
    match native_rule(driver, TransportDirection::Write, NativeElement::NotNull) {
        NativeRule::Preserved => crate::NullabilitySupport::Preserve,
        NativeRule::Transformed(_) | NativeRule::Rejected => {
            crate::NullabilitySupport::FormatDefined
        }
    }
}

/// Counts observations, with structural indices and no source values.
pub fn observe_native(
    report: &mut crate::LossReport,
    transport: (NativeDriver, TransportDirection),
    element: NativeElement,
    count: u64,
    layer: u64,
    field: Option<u64>,
) {
    if count == 0 {
        return;
    }
    let NativeRule::Transformed(change) = native_rule(transport.0, transport.1, element) else {
        return;
    };
    report.record(change.category(), count);
    report.add_example(crate::loss::LossExample {
        category: change.category().to_owned(),
        posizione: crate::loss::Posizione {
            layer_index: Some(layer), field_index: field, type_class: None,
        },
        context: match change {
            NativeChange::AltitudeMode => "non-absolute or implicit altitude mode discarded; Z is unconverted, not absolute elevation",
            NativeChange::CoordinatePrecision if transport.0 == NativeDriver::Dxf => "coordinates rounded to twelve decimal places, including signed zero normalization",
            NativeChange::CoordinatePrecision => "coordinates quantized on the native grid, including signed zero normalization",
            NativeChange::RingOrientation => "native ring orientation normalization; count is rings or polygonal geometries, not changed vertices",
            NativeChange::NumberPrecision => "native numeric representation converted to binary64; decimal precision is not retained",
            NativeChange::Dimensions => "dimension declaration reconstructed from native data; GeoJSON remains unknown",
            NativeChange::ExtendedData => "ExtendedData fields are emitted but not read, including SchemaData",
            _ => "native element transformed or omitted; structural indices only",
        }.to_owned() + &format!("; element={element:?}"),
    });
}

fn native_field_order(driver: NativeDriver, fields: &arrow_schema::Fields) -> Vec<usize> {
    let mut order: Vec<usize> = (0..fields.len()).collect();
    order.sort_by(|&a, &b| {
        let geometry_a = plenora_io_model::geometry::is_geometry_field(&fields[a]);
        let geometry_b = plenora_io_model::geometry::is_geometry_field(&fields[b]);
        geometry_b.cmp(&geometry_a).then_with(|| {
            if driver == NativeDriver::GeoJson {
                fields[a].name().cmp(fields[b].name())
            } else if driver == NativeDriver::Kml {
                let rank = |i: usize| match fields[i].name().as_str() {
                    "name" => 0,
                    "description" => 1,
                    _ => 2,
                };
                rank(a).cmp(&rank(b))
            } else {
                std::cmp::Ordering::Equal
            }
        })
    });
    order
}

/// Counts native geometry normalization units, not measured coordinate errors.
pub fn observe_geometry_write(
    report: &mut crate::LossReport,
    driver: NativeDriver,
    geometry: &plenora_io_model::wkb::WkbGeometry,
    layer: u64,
    field: u64,
) {
    use plenora_io_model::wkb::WkbValue;
    fn rings(value: &WkbValue) -> usize {
        match value {
            WkbValue::Polygon(rings) => rings.len(),
            WkbValue::MultiPolygon(children) | WkbValue::GeometryCollection(children) => {
                children.iter().map(|child| rings(&child.value)).sum()
            }
            _ => 0,
        }
    }
    let transport = (driver, TransportDirection::Write);
    let promoted = driver == NativeDriver::Shp
        && matches!(
            geometry.value,
            WkbValue::LineString(_) | WkbValue::Polygon(_)
        );
    let rings = rings(&geometry.value);
    observe_native(
        report,
        transport,
        NativeElement::GeometryType,
        u64::from(promoted),
        layer,
        Some(field),
    );
    observe_native(
        report,
        transport,
        NativeElement::RingOrientation,
        rings as u64,
        layer,
        Some(field),
    );
    observe_native(
        report,
        transport,
        NativeElement::CoordinatePrecision,
        1,
        layer,
        Some(field),
    );
}

/// Losses of the native schema that are known before the first batch.
#[must_use]
pub fn native_write_report(driver: NativeDriver, plan: &crate::WritePlan) -> crate::LossReport {
    let mut report = crate::LossReport::default();
    let transport = (driver, TransportDirection::Write);
    for (layer_index, layer) in plan.layers.iter().enumerate() {
        let fields = layer.contract.schema.fields();
        if driver == NativeDriver::Kml {
            for standard in ["name", "description"] {
                if !fields.iter().any(|field| field.name() == standard) {
                    observe_native(
                        &mut report,
                        transport,
                        NativeElement::FieldNames,
                        1,
                        layer_index as u64,
                        None,
                    );
                }
            }
        }
        let order = native_field_order(driver, fields);
        for (new_index, old_index) in order.iter().enumerate() {
            if new_index != *old_index {
                observe_native(
                    &mut report,
                    transport,
                    NativeElement::FieldOrder,
                    1,
                    layer_index as u64,
                    Some(*old_index as u64),
                );
            }
        }
        for (index, field) in fields.iter().enumerate() {
            let geometry = plenora_io_model::geometry::is_geometry_field(field);
            if !field.is_nullable() {
                observe_native(
                    &mut report,
                    transport,
                    NativeElement::NotNull,
                    1,
                    layer_index as u64,
                    Some(index as u64),
                );
            }
            if geometry {
                if field.name() != "geometry" {
                    observe_native(
                        &mut report,
                        transport,
                        NativeElement::FieldNames,
                        1,
                        layer_index as u64,
                        Some(index as u64),
                    );
                }
                if driver == NativeDriver::Kml
                    && layer.contract.geometry.as_ref().is_some_and(|g| {
                        g.geometry_types
                            .contains(&plenora_io_model::contract::GeometryType::GeometryCollection)
                    })
                {
                    observe_native(
                        &mut report,
                        transport,
                        NativeElement::GeometryType,
                        1,
                        layer_index as u64,
                        Some(index as u64),
                    );
                }
                observe_native(
                    &mut report,
                    transport,
                    NativeElement::Dimensions,
                    1,
                    layer_index as u64,
                    Some(index as u64),
                );
            } else if driver == NativeDriver::Kml
                && !matches!(field.name().as_str(), "name" | "description")
            {
                observe_native(
                    &mut report,
                    transport,
                    NativeElement::ExtendedData,
                    1,
                    layer_index as u64,
                    Some(index as u64),
                );
            }
        }
    }
    report
}

/// Attach observations from a complete native inference pass to its reader.
#[must_use]
pub fn with_native_report(
    inner: Box<dyn crate::LayerReader>,
    loss: crate::LossReport,
) -> Box<dyn crate::LayerReader> {
    Box::new(NativeReportReader { inner, loss })
}
struct NativeReportReader {
    inner: Box<dyn crate::LayerReader>,
    loss: crate::LossReport,
}
impl crate::LayerReader for NativeReportReader {
    fn contract(&self) -> &plenora_io_model::contract::LayerContract {
        self.inner.contract()
    }
    fn next_batch(&mut self) -> plenora_io_model::Result<Option<arrow_array::RecordBatch>> {
        self.inner.next_batch()
    }
    fn accepted_total(&self) -> Option<u64> {
        self.inner.accepted_total()
    }
    fn loss_report(&self) -> crate::LossReport {
        let mut loss = self.inner.loss_report();
        loss.merge(&self.loss);
        loss
    }
}

const SHP_NATIVE_CLASSES: [ArrowTypeClass; native_class_count(NativeDriver::Shp)] =
    native_classes(NativeDriver::Shp);

const DXF_NATIVE_CLASSES: [ArrowTypeClass; native_class_count(NativeDriver::Dxf)] =
    native_classes(NativeDriver::Dxf);

const FILEGDB_NATIVE_CLASSES: [ArrowTypeClass; native_class_count(NativeDriver::FileGdb)] =
    native_classes(NativeDriver::FileGdb);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const EXPECTED_NATIVE_ELEMENTS: &[&str] = &[
        "NullGeometry",
        "EmptyPoint",
        "SignedZero",
        "Subtypes",
        "Aliases",
        "Domains",
        "TextStyle",
        "PaperSpace",
        "Xdata",
        "Width",
        "Thickness",
        "Lineweight",
        "Linetype",
        "Color",
        "Handle",
        "DbfDate",
        "Utf8",
        "Null",
        "EmptyString",
        "SignedInteger",
        "UnsignedInteger",
        "Floating",
        "Boolean",
        "Binary",
        "Decimal",
        "Temporal",
        "NestedDictionary",
        "FieldNames",
        "NotNull",
        "FieldOrder",
        "SchemaMetadata",
        "FieldMetadata",
        "NativeMetadata",
        "Z",
        "M",
        "Dimensions",
        "GeometryType",
        "RingOrientation",
        "EmptyGeometry",
        "CoordinatePrecision",
        "CrsId",
        "CrsDefinition",
        "FeatureId",
        "Bbox",
        "ForeignMembers",
        "LayerMetadata",
        "Styles",
        "Hierarchy",
        "Extensions",
        "ExternalReference",
        "ExtendedData",
        "FeatureMetadata",
        "AltitudeMode",
        "Inference",
        "NumberPrecision",
        "StructuredValue",
        "MissingValue",
    ];

    #[test]
    fn native_matrix_enumerates_every_element_and_direction() {
        let qualified: Vec<_> = DRIVERS
            .iter()
            .filter_map(|row| row.qualified_native.map(|native| (row.driver, native)))
            .collect();
        assert_eq!(
            qualified,
            [
                ("shp", NativeDriver::Shp),
                ("filegdb", NativeDriver::FileGdb),
                ("geojson", NativeDriver::GeoJson),
                ("kml", NativeDriver::Kml),
                ("dxf", NativeDriver::Dxf)
            ]
        );
        for (id, driver) in &qualified {
            let catalog = rules(id).unwrap();
            for (class, element) in [
                (ArrowTypeClass::Boolean, NativeElement::Boolean),
                (ArrowTypeClass::SignedInteger, NativeElement::SignedInteger),
                (
                    ArrowTypeClass::UnsignedInteger,
                    NativeElement::UnsignedInteger,
                ),
                (ArrowTypeClass::Floating, NativeElement::Floating),
                (ArrowTypeClass::Utf8, NativeElement::Utf8),
                (ArrowTypeClass::Temporal, NativeElement::Temporal),
                (ArrowTypeClass::Binary, NativeElement::Binary),
                (ArrowTypeClass::Decimal, NativeElement::Decimal),
            ] {
                assert_eq!(
                    catalog.allowed_types.contains(&class),
                    native_rule(*driver, TransportDirection::Write, element)
                        != NativeRule::Rejected
                );
            }
            assert!(catalog.dimensions.contains(&CoordinateDimensions::Xyz));
            assert_eq!(
                native_rule(*driver, TransportDirection::Write, NativeElement::Z),
                NativeRule::Preserved
            );
            assert_eq!(
                catalog.dimensions.contains(&CoordinateDimensions::Xym),
                matches!(driver, NativeDriver::Shp | NativeDriver::FileGdb)
            );
            assert_eq!(
                catalog.dimensions.contains(&CoordinateDimensions::Xyzm),
                matches!(driver, NativeDriver::Shp | NativeDriver::FileGdb)
            );
            assert_eq!(
                native_rule(*driver, TransportDirection::Write, NativeElement::M),
                if matches!(driver, NativeDriver::Shp | NativeDriver::FileGdb) {
                    P
                } else {
                    R
                }
            );
        }
        let actual: Vec<_> = NATIVE_ELEMENTS
            .iter()
            .map(|element| format!("{element:?}"))
            .collect();
        assert_eq!(actual, EXPECTED_NATIVE_ELEMENTS);
        for driver in [
            NativeDriver::GeoJson,
            NativeDriver::Kml,
            NativeDriver::Shp,
            NativeDriver::Dxf,
            NativeDriver::FileGdb,
        ] {
            for direction in [TransportDirection::Read, TransportDirection::Write] {
                for &element in NATIVE_ELEMENTS {
                    let mut report = crate::LossReport::default();
                    observe_native(&mut report, (driver, direction), element, 3, 7, Some(11));
                    match native_rule(driver, direction, element) {
                        NativeRule::Transformed(change) => {
                            assert_eq!(report.counts[change.category()], 3);
                            let example = report.esempi_canonici().next().unwrap();
                            assert_eq!(example.posizione.layer_index, Some(7));
                            assert_eq!(example.posizione.field_index, Some(11));
                        }
                        NativeRule::Preserved | NativeRule::Rejected => assert!(report.is_empty()),
                    }
                }
            }
        }
    }

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
