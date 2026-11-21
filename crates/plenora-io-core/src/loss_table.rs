//! Regole di trasporto dello schema, condivise dal catalogo e dal piano di perdita.

mod applicability;
pub use applicability::{read_applicability, ReadApplicability};

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
        "field_alias",
        "plenora.field.alias",
        Rule::Removed,
        Rule::Preserved,
    ),
    (
        "field_domain",
        "plenora.field.domain",
        Rule::Removed,
        Rule::Preserved,
    ),
    (
        "domains",
        "plenora.dataset.domains",
        Rule::Removed,
        Rule::Preserved,
    ),
    (
        "subtypes",
        "plenora.layer.subtypes",
        Rule::Removed,
        Rule::Preserved,
    ),
    (
        "layer_alias",
        "plenora.layer.alias",
        Rule::Removed,
        Rule::Preserved,
    ),
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
    ("", "gpkg.identifier", Rule::Preserved, Rule::Preserved),
    ("", "gpkg.description", Rule::Preserved, Rule::Preserved),
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

pub const CSV: DriverRules = DriverRules {
    driver: "csv",
    qualified_native: Some(NativeDriver::Csv),
    dimensions: native_dimensions(NativeDriver::Csv),
    allowed_types: &CSV_NATIVE_CLASSES,
    stable_types: &[DataType::Utf8],
    metadata: MetadataTransport::Native,
};
pub const SHP: DriverRules = DriverRules {
    driver: "shp",
    qualified_native: Some(NativeDriver::Shp),
    dimensions: native_dimensions(NativeDriver::Shp),
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
    qualified_native: Some(NativeDriver::Gpkg),
    dimensions: native_dimensions(NativeDriver::Gpkg),
    allowed_types: &GPKG_NATIVE_CLASSES,
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
    dimensions: native_dimensions(NativeDriver::FileGdb),
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
    dimensions: native_dimensions(NativeDriver::GeoJson),
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
    dimensions: native_dimensions(NativeDriver::Kml),
    allowed_types: &KML_NATIVE_CLASSES,
    stable_types: &[DataType::Utf8],
    metadata: MetadataTransport::Native,
};
pub const DXF: DriverRules = DriverRules {
    driver: "dxf",
    qualified_native: Some(NativeDriver::Dxf),
    dimensions: native_dimensions(NativeDriver::Dxf),
    allowed_types: &DXF_NATIVE_CLASSES,
    stable_types: &[],
    metadata: MetadataTransport::Native,
};
pub const XLS: DriverRules = DriverRules {
    driver: "xls",
    qualified_native: Some(NativeDriver::Xls),
    dimensions: native_dimensions(NativeDriver::Xls),
    allowed_types: &XLS_NATIVE_CLASSES,
    stable_types: &[DataType::Float64, DataType::Boolean, DataType::Utf8],
    metadata: MetadataTransport::Native,
};
pub const GEOPARQUET: DriverRules = DriverRules {
    driver: "geoparquet",
    qualified_native: Some(NativeDriver::GeoParquet),
    dimensions: native_dimensions(NativeDriver::GeoParquet),
    allowed_types: &GEOPARQUET_NATIVE_CLASSES,
    stable_types: &[],
    metadata: MetadataTransport::Arrow,
};
pub const IPC: DriverRules = DriverRules {
    driver: "ipc",
    qualified_native: Some(NativeDriver::Ipc),
    dimensions: native_dimensions(NativeDriver::Ipc),
    allowed_types: &IPC_NATIVE_CLASSES,
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
        Some(MetadataTransport::Native | MetadataTransport::Arrow) | None => Rule::Preserved,
    }
}

/// Native observations use the same rules as schema transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeChange {
    DomainNotTransported,
    SubtypeNotTransported,
    AliasNotTransported,
    GpkgMetadata,
    CrsIdAbsent,
    SqlSchema,
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
const NATIVE_DOMAIN: &str = "native_domain_not_transported";
const NATIVE_SUBTYPE: &str = "native_subtype_not_transported";
const NATIVE_ALIAS: &str = "native_alias_not_transported";
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
            Self::DomainNotTransported => NATIVE_DOMAIN,
            Self::SubtypeNotTransported => NATIVE_SUBTYPE,
            Self::AliasNotTransported => NATIVE_ALIAS,
            Self::GpkgMetadata => "gpkg_metadata_not_preserved",
            Self::CrsIdAbsent => "crs_id_not_preserved_absent",
            Self::SqlSchema => "gpkg_schema_normalized",
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
    Csv,
    Xls,
    Gpkg,
    GeoParquet,
    Ipc,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeRule {
    Preserved,
    Transformed(NativeChange),
    Rejected,
}

macro_rules! native_elements {
    ($($element:ident => [$gr:expr, $gw:expr, $kr:expr, $kw:expr, $sr:expr, $sw:expr, $dr:expr, $dw:expr, $fr:expr, $fw:expr, $cr:expr, $cw:expr, $xr:expr, $xw:expr, $pr:expr, $pw:expr, $qr:expr, $qw:expr, $ir:expr, $iw:expr];)+) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum NativeElement { $($element,)+ }
        pub const NATIVE_ELEMENTS: &[NativeElement] = &[$(NativeElement::$element,)+];
        impl NativeElement {
            #[must_use]
            pub const fn rules(self) -> [NativeRule; 20] {
                match self { $(Self::$element => [$gr, $gw, $kr, $kw, $sr, $sw, $dr, $dw, $fr, $fw, $cr, $cw, $xr, $xw, $pr, $pw, $qr, $qw, $ir, $iw],)+ }
            }
        }
    };
}
use NativeRule::{Preserved as P, Rejected as R, Transformed as T};
native_elements! {
    NullGeometry => [P, P, P, P, P, P, R, R, P, P, P, P, P, P, P, P, P, P, P, P];
    EmptyPoint => [R, R, R, R, P, P, R, R, T(NativeChange::MissingValue), T(NativeChange::MissingValue), R, R, R, R, P, P, P, P, P, P];
    SignedZero => [P, P, P, P, P, P, P, T(NativeChange::CoordinatePrecision), P, T(NativeChange::CoordinatePrecision), P, P, P, P, P, P, P, P, P, P];
    Subtypes => [R, T(NativeChange::SubtypeNotTransported), R, T(NativeChange::SubtypeNotTransported), R, T(NativeChange::SubtypeNotTransported), R, T(NativeChange::SubtypeNotTransported), P, T(NativeChange::SubtypeNotTransported), R, T(NativeChange::SubtypeNotTransported), R, T(NativeChange::SubtypeNotTransported), R, T(NativeChange::SubtypeNotTransported), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    Aliases => [R, T(NativeChange::AliasNotTransported), R, T(NativeChange::AliasNotTransported), R, T(NativeChange::AliasNotTransported), R, T(NativeChange::AliasNotTransported), P, T(NativeChange::AliasNotTransported), R, T(NativeChange::AliasNotTransported), R, T(NativeChange::AliasNotTransported), R, T(NativeChange::AliasNotTransported), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    Domains => [R, T(NativeChange::DomainNotTransported), R, T(NativeChange::DomainNotTransported), R, T(NativeChange::DomainNotTransported), R, T(NativeChange::DomainNotTransported), P, T(NativeChange::DomainNotTransported), R, T(NativeChange::DomainNotTransported), R, T(NativeChange::DomainNotTransported), R, T(NativeChange::DomainNotTransported), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    TextStyle => [R, R, R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R, R, R];
    PaperSpace => [R, R, R, R, R, R, T(NativeChange::NativeHierarchy), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R, R, R];
    Xdata => [R, R, R, R, R, R, T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R, R, R];
    Width => [R, R, R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R, R, R];
    Thickness => [R, R, R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R, R, R];
    Lineweight => [R, R, R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R, R, R];
    Linetype => [R, R, R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R, R, R];
    Color => [R, R, R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R, R, R];
    Handle => [R, R, R, R, R, R, T(NativeChange::FeatureId), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R, R, R];
    DbfDate => [R, R, R, R, T(NativeChange::ArrowCoercion), T(NativeChange::ArrowCoercion), R, R, R, R, R, R, R, R, R, R, R, R, R, R];
    Utf8 => [P, P, P, P, P, P, P, T(NativeChange::DxfAttribute), P, P, P, P, P, P, P, P, P, P, P, P];
    Null => [P, P, P, P, P, P, P, T(NativeChange::DxfAttribute), P, P, P, P, P, P, P, P, P, P, P, P];
    EmptyString => [P, P, P, P, R, R, P, T(NativeChange::DxfAttribute), P, P, P, P, P, P, P, P, P, P, P, P];
    SignedInteger => [T(NativeChange::Inference), T(NativeChange::ArrowCoercion), R, T(NativeChange::ArrowCoercion), T(NativeChange::Inference), T(NativeChange::ArrowCoercion), R, T(NativeChange::DxfAttribute), P, T(NativeChange::ArrowCoercion), T(NativeChange::Inference), T(NativeChange::ArrowCoercion), T(NativeChange::Inference), T(NativeChange::ArrowCoercion), P, T(NativeChange::ArrowCoercion), P, P, P, P];
    UnsignedInteger => [T(NativeChange::Inference), T(NativeChange::ArrowCoercion), R, T(NativeChange::ArrowCoercion), R, T(NativeChange::ArrowCoercion), R, T(NativeChange::DxfAttribute), R, R, T(NativeChange::Inference), T(NativeChange::ArrowCoercion), T(NativeChange::Inference), T(NativeChange::ArrowCoercion), P, T(NativeChange::ArrowCoercion), P, P, P, P];
    Floating => [T(NativeChange::NumberPrecision), T(NativeChange::ArrowCoercion), R, T(NativeChange::ArrowCoercion), P, T(NativeChange::ArrowCoercion), R, T(NativeChange::DxfAttribute), P, T(NativeChange::ArrowCoercion), T(NativeChange::Inference), T(NativeChange::ArrowCoercion), T(NativeChange::Inference), T(NativeChange::ArrowCoercion), P, T(NativeChange::ArrowCoercion), P, P, P, P];
    Boolean => [P, P, R, T(NativeChange::ArrowCoercion), P, P, R, T(NativeChange::DxfAttribute), R, R, P, T(NativeChange::ArrowCoercion), P, P, P, T(NativeChange::ArrowCoercion), P, P, P, P];
    Binary => [R, R, R, R, R, R, R, T(NativeChange::DxfAttribute), P, R, R, R, R, R, P, P, P, P, P, P];
    Decimal => [R, R, R, R, R, R, R, T(NativeChange::DxfAttribute), R, R, R, R, R, R, R, R, P, P, P, P];
    Temporal => [R, T(NativeChange::ArrowCoercion), R, T(NativeChange::ArrowCoercion), T(NativeChange::ArrowCoercion), T(NativeChange::ArrowCoercion), R, T(NativeChange::DxfAttribute), T(NativeChange::NumberPrecision), R, T(NativeChange::Inference), T(NativeChange::ArrowCoercion), T(NativeChange::Inference), T(NativeChange::ArrowCoercion), T(NativeChange::ArrowCoercion), R, P, P, P, P];
    NestedDictionary => [T(NativeChange::StructuredValue), R, R, R, R, R, R, T(NativeChange::DxfAttribute), R, R, R, R, R, R, R, R, P, P, P, P];
    FieldNames => [P, T(NativeChange::FieldNames), T(NativeChange::ExtendedData), T(NativeChange::FieldNames), P, T(NativeChange::FieldNames), T(NativeChange::FieldNames), T(NativeChange::FieldNames), P, T(NativeChange::FieldNames), T(NativeChange::FieldNames), T(NativeChange::FieldNames), T(NativeChange::FieldNames), T(NativeChange::FieldNames), P, P, P, P, P, P];
    NotNull => [P, T(NativeChange::NotNull), P, T(NativeChange::NotNull), P, T(NativeChange::NotNull), P, T(NativeChange::NotNull), T(NativeChange::NotNull), T(NativeChange::NotNull), P, T(NativeChange::NotNull), P, T(NativeChange::NotNull), P, P, P, P, P, P];
    FieldOrder => [T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::FieldOrder), T(NativeChange::SqlSchema), T(NativeChange::SqlSchema), P, P, P, P];
    SchemaMetadata => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    FieldMetadata => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    NativeMetadata => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    Z => [P, P, T(NativeChange::AltitudeMode), P, P, P, P, P, P, P, P, P, P, P, P, P, P, P, P, P];
    M => [R, R, R, R, P, P, R, R, P, P, P, P, P, P, P, P, R, R, P, P];
    Dimensions => [T(NativeChange::Dimensions), T(NativeChange::Dimensions), T(NativeChange::Dimensions), T(NativeChange::Dimensions), P, P, T(NativeChange::Dimensions), T(NativeChange::Dimensions), P, P, P, P, P, P, P, P, P, P, P, P];
    GeometryType => [P, P, T(NativeChange::GeometryType), T(NativeChange::GeometryType), T(NativeChange::GeometryType), T(NativeChange::GeometryType), T(NativeChange::GeometryType), T(NativeChange::GeometryType), P, P, P, P, P, P, P, P, P, P, P, P];
    RingOrientation => [P, P, P, P, P, T(NativeChange::RingOrientation), P, P, P, T(NativeChange::RingOrientation), P, P, P, P, P, P, P, P, P, P];
    EmptyGeometry => [R, R, R, R, R, R, R, R, T(NativeChange::MissingValue), T(NativeChange::MissingValue), P, P, P, P, P, P, P, P, P, P];
    CoordinatePrecision => [T(NativeChange::NumberPrecision), P, T(NativeChange::NumberPrecision), P, P, P, P, T(NativeChange::CoordinatePrecision), P, T(NativeChange::CoordinatePrecision), T(NativeChange::NumberPrecision), P, T(NativeChange::NumberPrecision), P, P, P, P, P, P, P];
    CrsId => [P, T(NativeChange::CrsId), P, T(NativeChange::CrsId), P, T(NativeChange::CrsId), P, T(NativeChange::CrsId), P, T(NativeChange::CrsId), P, T(NativeChange::CrsIdAbsent), P, T(NativeChange::CrsIdAbsent), P, P, P, P, P, P];
    CrsDefinition => [P, T(NativeChange::CrsDefinition), P, T(NativeChange::CrsDefinition), P, T(NativeChange::CrsDefinition), P, T(NativeChange::CrsDefinition), P, T(NativeChange::CrsDefinition), P, T(NativeChange::CrsDefinition), P, T(NativeChange::CrsDefinition), P, P, P, P, P, P];
    FeatureId => [T(NativeChange::FeatureId), T(NativeChange::ArrowMetadata), T(NativeChange::FeatureId), T(NativeChange::ArrowMetadata), T(NativeChange::FeatureId), T(NativeChange::ArrowMetadata), T(NativeChange::FeatureId), T(NativeChange::ArrowMetadata), T(NativeChange::FeatureId), T(NativeChange::ArrowMetadata), P, P, P, P, P, P, P, P, P, P];
    Bbox => [T(NativeChange::Bbox), T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata), T(NativeChange::Bbox), T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata), T(NativeChange::Bbox), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, T(NativeChange::Bbox), T(NativeChange::ArrowMetadata), R, R];
    ForeignMembers => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R];
    LayerMetadata => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), R, R, R, R, T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), R, R, R, R];
    Styles => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::Styles), T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata), T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, T(NativeChange::ArrowMetadata), R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R, R, R];
    Hierarchy => [R, R, T(NativeChange::Hierarchy), T(NativeChange::ArrowMetadata), R, R, T(NativeChange::NativeHierarchy), T(NativeChange::ArrowMetadata), T(NativeChange::NativeHierarchy), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R];
    Extensions => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), R, R, R, R, T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), R, R, R, R];
    ExternalReference => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::ExternalReference), T(NativeChange::ArrowMetadata), R, R, T(NativeChange::ExternalReference), T(NativeChange::ArrowMetadata), T(NativeChange::ExternalReference), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R];
    ExtendedData => [R, R, T(NativeChange::ExtendedData), T(NativeChange::ExtendedData), R, R, T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R, R, R];
    FeatureMetadata => [T(NativeChange::ForeignMembers), T(NativeChange::ArrowMetadata), T(NativeChange::FeatureMetadata), T(NativeChange::ArrowMetadata), R, R, T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), R, R, R, R, R, R, R, R, R, R, R, R];
    AltitudeMode => [R, R, T(NativeChange::AltitudeMode), P, R, R, R, R, R, R, R, R, R, R, R, R, R, R, R, R];
    Inference => [T(NativeChange::Inference), T(NativeChange::Inference), R, T(NativeChange::Inference), T(NativeChange::Inference), P, R, R, R, R, T(NativeChange::Inference), T(NativeChange::Inference), T(NativeChange::Inference), T(NativeChange::Inference), R, R, R, R, R, R];
    NumberPrecision => [T(NativeChange::NumberPrecision), P, T(NativeChange::NumberPrecision), P, P, P, P, T(NativeChange::CoordinatePrecision), P, T(NativeChange::CoordinatePrecision), T(NativeChange::NumberPrecision), P, T(NativeChange::NumberPrecision), P, P, P, P, P, P, P];
    StructuredValue => [T(NativeChange::StructuredValue), R, T(NativeChange::ExtendedData), R, R, R, R, R, R, R, R, R, R, R, R, R, R, R, R, R];
    MissingValue => [T(NativeChange::MissingValue), P, T(NativeChange::MissingValue), P, P, P, P, P, P, P, R, R, R, R, R, R, R, R, R, R];
    CoordinateColumns => [R, R, R, R, R, R, R, R, R, R, T(NativeChange::FieldNames), T(NativeChange::FieldNames), T(NativeChange::FieldNames), T(NativeChange::FieldNames), R, R, R, R, R, R];
    NumericFormat => [R, R, R, R, R, R, R, R, R, R, R, R, T(NativeChange::NativeStyles), T(NativeChange::ArrowMetadata), R, R, R, R, R, R];
    Formula => [R, R, R, R, R, R, R, R, R, R, R, R, T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), R, R, R, R, R, R];
    FormulaCache => [R, R, R, R, R, R, R, R, R, R, R, R, P, P, R, R, R, R, R, R];
    Identifier => [R, R, R, R, R, R, R, R, R, R, R, R, R, R, P, P, R, R, R, R];
    Description => [R, R, R, R, R, R, R, R, R, R, R, R, R, R, P, P, R, R, R, R];
    DataColumns => [R, R, R, R, R, R, R, R, R, R, R, R, R, R, T(NativeChange::LayerMetadata), T(NativeChange::ArrowMetadata), R, R, R, R];
    SqlConstraints => [R, R, R, R, R, R, R, R, R, R, R, R, R, R, T(NativeChange::SqlSchema), T(NativeChange::SqlSchema), R, R, R, R];
    ContractVersion => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, P, T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    FieldIdentity => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, P, T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    TypesDeclaration => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    GeometryEncoding => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    SpatialSemantics => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    Precision => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    Srid => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    CrsResolution => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    DefinitionFormat => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    AxisOrder => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    PrimaryGeometry => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    ExtensionName => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    ExtensionMetadata => [P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::ArrowMetadata), P, T(NativeChange::GpkgMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata), T(NativeChange::ArrowMetadata)];
    UnknownDimensions => [P, P, P, P, P, P, P, P, R, R, P, P, P, P, P, P, P, P, P, P];
    Covering => [R, R, R, R, R, R, R, R, R, R, R, R, R, R, R, R, T(NativeChange::Bbox), P, R, R];
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
        (NativeDriver::Csv, TransportDirection::Read) => 10,
        (NativeDriver::Csv, TransportDirection::Write) => 11,
        (NativeDriver::Xls, TransportDirection::Read) => 12,
        (NativeDriver::Xls, TransportDirection::Write) => 13,
        (NativeDriver::Gpkg, TransportDirection::Read) => 14,
        (NativeDriver::Gpkg, TransportDirection::Write) => 15,
        (NativeDriver::GeoParquet, TransportDirection::Read) => 16,
        (NativeDriver::GeoParquet, TransportDirection::Write) => 17,
        (NativeDriver::Ipc, TransportDirection::Read) => 18,
        (NativeDriver::Ipc, TransportDirection::Write) => 19,
    };
    element.rules()[index]
}

/// Rule for an Arrow fixture carrying a foreign native element into a writer.
///
/// Native observations retain `native_rule`: e.g. omitted `ExtendedData` remains
/// an `ExtendedData` loss, independently of an opaque foreign metadata envelope.
#[must_use]
pub const fn native_write_transport_rule(
    driver: NativeDriver,
    element: NativeElement,
) -> NativeRule {
    let direction = TransportDirection::Write;
    if matches!(
        element,
        NativeElement::Domains | NativeElement::Aliases | NativeElement::Subtypes
    ) {
        return native_rule(driver, direction, element);
    }
    // Foreign native objects reach a writer through Arrow, not through the
    // destination grammar. Their transport follows metadata/scalar rules.
    if matches!(direction, TransportDirection::Write) {
        use NativeElement as E;
        let transport = match element {
            E::Subtypes
            | E::Aliases
            | E::Domains
            | E::TextStyle
            | E::PaperSpace
            | E::Xdata
            | E::Width
            | E::Thickness
            | E::Lineweight
            | E::Linetype
            | E::Color
            | E::Handle
            | E::FeatureId
            | E::Bbox
            | E::ForeignMembers
            | E::LayerMetadata
            | E::Styles
            | E::Hierarchy
            | E::Extensions
            | E::ExternalReference
            | E::ExtendedData
            | E::FeatureMetadata
            | E::AltitudeMode
            | E::CoordinateColumns
            | E::NumericFormat
            | E::Formula
            | E::DataColumns
            | E::SqlConstraints
            | E::Covering => Some(E::NativeMetadata),
            E::Identifier | E::Description => Some(E::SchemaMetadata),
            E::DbfDate | E::StructuredValue => Some(E::Utf8),
            E::FormulaCache => Some(E::Floating),
            E::Inference => Some(E::SignedInteger),
            E::MissingValue => Some(E::Null),
            _ => None,
        };
        if let Some(transport) = transport {
            return native_rule(driver, direction, transport);
        }
    }
    native_rule(driver, direction, element)
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
            NativeChange::RingOrientation => "native ring orientation differs from the source",
            NativeChange::NumberPrecision => "reconstructed numeric value differs from the source decimal",
            NativeChange::Dimensions => "dimension declaration reconstructed from native data; GeoJSON remains unknown",
            NativeChange::ExtendedData => "ExtendedData fields are emitted but not read, including SchemaData",
            _ => "native element transformed or omitted; structural indices only",
        }.to_owned() + &format!("; element={element:?}"),
    });
}

/// Residual attribute loss after a native reader tried to transport a definition.
/// The enum, never the diagnostic prose, identifies the loss category.
pub fn observe_attribute_loss(
    report: &mut crate::LossReport,
    element: NativeElement,
    count: u64,
    layer: u64,
) {
    let change = match element {
        NativeElement::Domains => NativeChange::DomainNotTransported,
        NativeElement::Subtypes => NativeChange::SubtypeNotTransported,
        NativeElement::Aliases => NativeChange::AliasNotTransported,
        _ => return,
    };
    if count == 0 {
        return;
    }
    report.record(change.category(), count);
    report.add_example(crate::loss::LossExample {
        category: change.category().to_owned(),
        posizione: crate::loss::Posizione {
            layer_index: Some(layer),
            ..crate::loss::Posizione::default()
        },
        context: "metadato nativo non trasportato".to_owned(),
    });
}

/// Counts geometry changes by comparing the native reconstruction with input values.
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
            WkbValue::Polygon(rings) => rings
                .iter()
                .enumerate()
                .filter(|(index, ring)| {
                    let area: f64 = ring
                        .windows(2)
                        .map(|pair| pair[1].x.mul_add(-pair[0].y, pair[0].x * pair[1].y))
                        .sum();
                    // Shapefile stores clockwise exteriors and counterclockwise holes.
                    if *index == 0 {
                        area > 0.0
                    } else {
                        area < 0.0
                    }
                })
                .count(),
            WkbValue::MultiPolygon(children) | WkbValue::GeometryCollection(children) => {
                children.iter().map(|child| rings(&child.value)).sum()
            }
            _ => 0,
        }
    }
    fn rounded(value: &WkbValue) -> bool {
        let coordinate = |c: &plenora_io_model::wkb::WkbCoordinate| {
            [Some(c.x), Some(c.y), c.z].into_iter().flatten().any(|v| {
                let reconstructed = format!("{v:.12}").parse::<f64>().unwrap_or(v);
                let reconstructed = if reconstructed == 0.0 {
                    0.0
                } else {
                    reconstructed
                };
                reconstructed.to_bits() != v.to_bits()
            })
        };
        match value {
            WkbValue::Point(c) => coordinate(c),
            WkbValue::LineString(c) => c.iter().any(coordinate),
            WkbValue::Polygon(rings) => rings.iter().flatten().any(coordinate),
            WkbValue::MultiPoint(children)
            | WkbValue::MultiLineString(children)
            | WkbValue::MultiPolygon(children)
            | WkbValue::GeometryCollection(children) => children.iter().any(|g| rounded(&g.value)),
            _ => false,
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
        u64::from(driver == NativeDriver::Dxf && rounded(&geometry.value)),
        layer,
        Some(field),
    );
}

/// Empty initial report; native schema losses require completed-output evidence.
#[must_use]
pub fn native_write_report(_driver: NativeDriver, _plan: &crate::WritePlan) -> crate::LossReport {
    // Initialization only. The native reader supplies evidence at finalization.
    crate::LossReport::default()
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

const CSV_NATIVE_CLASSES: [ArrowTypeClass; native_class_count(NativeDriver::Csv)] =
    native_classes(NativeDriver::Csv);

const XLS_NATIVE_CLASSES: [ArrowTypeClass; native_class_count(NativeDriver::Xls)] =
    native_classes(NativeDriver::Xls);

const GPKG_NATIVE_CLASSES: [ArrowTypeClass; native_class_count(NativeDriver::Gpkg)] =
    native_classes(NativeDriver::Gpkg);

const GEOPARQUET_NATIVE_CLASSES: [ArrowTypeClass; native_class_count(NativeDriver::GeoParquet)] =
    native_classes(NativeDriver::GeoParquet);

const IPC_NATIVE_CLASSES: [ArrowTypeClass; native_class_count(NativeDriver::Ipc)] =
    native_classes(NativeDriver::Ipc);

/// Reports changed or removed source metadata keys, never their contents.
#[must_use]
pub fn metadata_changes(
    driver: NativeDriver,
    direction: TransportDirection,
    before: &arrow_schema::Schema,
    after: &arrow_schema::Schema,
) -> crate::LossReport {
    let mut report = crate::LossReport::default();
    let mut compare = |before: &arrow_schema::Metadata,
                       after: &arrow_schema::Metadata,
                       field: Option<u64>| {
        for (key, value) in before {
            if after.get(key) == Some(value) {
                continue;
            }
            let element = match key.as_str() {
                "plenora.contract.version" => NativeElement::ContractVersion,
                "plenora.field.alias" | "plenora.layer.alias" => NativeElement::Aliases,
                "plenora.field.domain" | "plenora.dataset.domains" => NativeElement::Domains,
                "plenora.layer.subtypes" => NativeElement::Subtypes,
                "plenora.field_id" => NativeElement::FieldIdentity,
                "plenora.geometry.types_declaration" => NativeElement::TypesDeclaration,
                "plenora.geometry.encoding" => NativeElement::GeometryEncoding,
                "plenora.geometry.spatial_semantics" => NativeElement::SpatialSemantics,
                "plenora.geometry.precision" => NativeElement::Precision,
                "plenora.geometry.srid" => NativeElement::Srid,
                "plenora.geometry.crs_resolution" => NativeElement::CrsResolution,
                "plenora.geometry.crs_definition_format" => NativeElement::DefinitionFormat,
                "plenora.geometry.axis_order" => NativeElement::AxisOrder,
                "plenora.primary_geometry" => NativeElement::PrimaryGeometry,
                "ARROW:extension:name" => NativeElement::ExtensionName,
                "ARROW:extension:metadata" => NativeElement::ExtensionMetadata,
                _ if key.starts_with("plenora.geometry.native.") => NativeElement::NativeMetadata,
                _ if field.is_some() => NativeElement::FieldMetadata,
                _ => NativeElement::SchemaMetadata,
            };
            observe_native(&mut report, (driver, direction), element, 1, 0, field);
        }
    };
    compare(before.metadata(), after.metadata(), None);
    for (index, field) in before.fields().iter().enumerate() {
        let output = after.field_with_name(field.name()).ok().or_else(|| {
            plenora_io_model::geometry::is_geometry_field(field)
                .then(|| {
                    after
                        .fields()
                        .iter()
                        .find(|f| plenora_io_model::geometry::is_geometry_field(f))
                        .map(AsRef::as_ref)
                })
                .flatten()
        });
        let empty = arrow_schema::Metadata::new();
        compare(
            field.metadata(),
            output.map_or(&empty, arrow_schema::Field::metadata),
            Some(index as u64),
        );
    }
    report
}

/// Tabular geometry reconstruction, counted per source column.
#[must_use]
pub fn tabular_geometry_report(
    driver: NativeDriver,
    headers: &[String],
    columns: &[usize],
    xy: bool,
    layer: u64,
) -> crate::LossReport {
    let mut report = crate::LossReport::default();
    for &index in columns {
        if xy || headers[index] != "geometry" {
            observe_native(
                &mut report,
                (driver, TransportDirection::Read),
                if xy {
                    NativeElement::CoordinateColumns
                } else {
                    NativeElement::FieldNames
                },
                1,
                layer,
                Some(index as u64),
            );
        }
    }
    if columns.first().is_some_and(|&i| i != 0) {
        observe_native(
            &mut report,
            (driver, TransportDirection::Read),
            NativeElement::FieldOrder,
            1,
            layer,
            None,
        );
    }
    report
}

const fn native_dimensions(driver: NativeDriver) -> &'static [CoordinateDimensions] {
    if matches!(
        native_rule(driver, TransportDirection::Write, NativeElement::M),
        NativeRule::Rejected
    ) {
        XYZ_DIMENSIONS
    } else if matches!(
        native_rule(
            driver,
            TransportDirection::Write,
            NativeElement::UnknownDimensions
        ),
        NativeRule::Rejected
    ) {
        GDB_DIMENSIONS
    } else {
        ALL_DIMENSIONS
    }
}

#[cfg(test)]
mod tests;
