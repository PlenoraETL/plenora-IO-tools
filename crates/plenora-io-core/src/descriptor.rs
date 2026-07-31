//! `FormatDescriptor` — catalogo machine-readable dei driver (Architetture §2.3).

use serde::Serialize;

use plenora_io_model::contract::{
    CoordinateDimensions, GeometryEncoding, GeometryType, SpatialSemantics,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Read,
    Write,
    Bidirectional,
}

/// Modalità di lettura, per-driver e per-versione (D9): non permanente.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadMode {
    StreamingSequential,
    StreamingColumnar,
    Materializing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteMode {
    Streaming,
    Buffered,
}

/// Livello di determinismo garantito a parità di input, opzioni e versione
/// dell'implementazione (ICD §12).
///
/// `Semantic` è la garanzia minima: stessi valori e stesso insieme di righe,
/// senza assumere un ordine o una rappresentazione fisica identici.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeterminismLevel {
    Semantic,
    Ordered,
    ByteForByte,
    Unordered,
}

/// Fedeltà a tre livelli: dipende dal contratto, non solo dal formato (ADR-IO 5).
/// Il descrittore porta la capacità generale; `open`/`create` la valutazione
/// specifica.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    Lossless,
    Conditional,
    Approximating,
}

/// Concorrenza dei reader (ADR-IO 1): più espressiva di un bool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderConcurrency {
    SingleActiveReader,
    MultipleIndependentReaders,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    PureRust,
    Gdal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrsHandling {
    Embedded,
    FixedWgs84,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextEncoding {
    Utf8,
    Ascii,
    FormatDefined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NameNormalization {
    None,
    Nfc,
    FormatDefined,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct FieldNamePolicy {
    pub max_bytes: Option<usize>,
    pub max_chars: Option<usize>,
    pub encoding: TextEncoding,
    pub case_sensitive: bool,
    pub normalization: NameNormalization,
    pub reserved_names: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrowTypeClass {
    Boolean,
    SignedInteger,
    UnsignedInteger,
    Floating,
    Utf8,
    Binary,
    Temporal,
    Decimal,
    Nested,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeCoercionPolicy {
    Reject,
    ExplicitText,
    LossReported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeWriteSupport {
    All,
    NamedSubset(&'static [&'static str]),
    None,
    LossReported,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct GeometryWriteSupport {
    pub supported: bool,
    pub encodings: &'static [GeometryEncoding],
    pub dimensions: &'static [CoordinateDimensions],
    pub spatial_semantics: &'static [SpatialSemantics],
    /// Tipi geometrici accettati dal formato nel profilo corrente.
    pub geometry_types: &'static [GeometryType],
    pub mixed_types: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrsWriteSupport {
    /// Il formato richiede un CRS risolto e lo incorpora.
    Embedded,
    /// Il formato incorpora senza perdita anche gli stati dichiarato ma non
    /// risolto e assente.
    EmbeddedOptional,
    Fixed(&'static str),
    None,
}

/// Esito di una rappresentazione CRS presente nel contratto quando attraversa
/// un writer. Solo `Preserved` conserva il valore sorgente in modo
/// indipendente; `Derived` indica che il formato o il driver ricostruisce il
/// valore da un'altra rappresentazione.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrsRepresentationState {
    Preserved,
    Absent,
    Derived,
}

/// Capability generale del writer per le tre rappresentazioni CRS del
/// contratto. È distinta da [`CrsWriteSupport`]: quest'ultima stabilisce se il
/// formato richiede/incorpora un CRS, questa struttura stabilisce cosa
/// sopravvive davvero al bordo di scrittura.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CrsRepresentationCapabilities {
    pub crs_id: CrsRepresentationState,
    pub srid: CrsRepresentationState,
    pub crs_definition: CrsRepresentationState,
}

impl CrsRepresentationCapabilities {
    pub const fn new(
        crs_id: CrsRepresentationState,
        srid: CrsRepresentationState,
        crs_definition: CrsRepresentationState,
    ) -> Self {
        Self {
            crs_id,
            srid,
            crs_definition,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NullabilitySupport {
    Preserve,
    FormatDefined,
    NoNulls,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct FormatWriteCapabilities {
    pub field_names: FieldNamePolicy,
    pub allowed_types: &'static [ArrowTypeClass],
    pub type_coercion: TypeCoercionPolicy,
    pub attributes: AttributeWriteSupport,
    pub geometry: GeometryWriteSupport,
    pub crs: CrsWriteSupport,
    pub crs_representations: CrsRepresentationCapabilities,
    pub nullability: NullabilitySupport,
    pub multi_layer: bool,
}

pub const UTF8_FIELD_NAMES: FieldNamePolicy = FieldNamePolicy {
    max_bytes: None,
    max_chars: None,
    encoding: TextEncoding::Utf8,
    case_sensitive: true,
    normalization: NameNormalization::None,
    reserved_names: &[],
};

pub const DBF_FIELD_NAMES: FieldNamePolicy = FieldNamePolicy {
    max_bytes: Some(10),
    max_chars: Some(10),
    encoding: TextEncoding::Ascii,
    case_sensitive: false,
    normalization: NameNormalization::FormatDefined,
    reserved_names: &[],
};

pub const SCALAR_TYPES: &[ArrowTypeClass] = &[
    ArrowTypeClass::Boolean,
    ArrowTypeClass::SignedInteger,
    ArrowTypeClass::UnsignedInteger,
    ArrowTypeClass::Floating,
    ArrowTypeClass::Utf8,
    ArrowTypeClass::Binary,
    ArrowTypeClass::Temporal,
    ArrowTypeClass::Decimal,
];

pub const ALL_ARROW_TYPES: &[ArrowTypeClass] = &[
    ArrowTypeClass::Boolean,
    ArrowTypeClass::SignedInteger,
    ArrowTypeClass::UnsignedInteger,
    ArrowTypeClass::Floating,
    ArrowTypeClass::Utf8,
    ArrowTypeClass::Binary,
    ArrowTypeClass::Temporal,
    ArrowTypeClass::Decimal,
    ArrowTypeClass::Nested,
    ArrowTypeClass::Other,
];

pub const ALL_GEOMETRY_TYPES: &[GeometryType] = &[
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

/// Sette tipi semplici decodificati dal codec WKB locale. È distinto
/// dall'universo normativo R3.1: conoscere un tipo nel contratto non implica
/// che un driver sappia materializzarlo.
pub const SIMPLE_WKB_GEOMETRY_TYPES: &[GeometryType] = &[
    GeometryType::Point,
    GeometryType::LineString,
    GeometryType::Polygon,
    GeometryType::MultiPoint,
    GeometryType::MultiLineString,
    GeometryType::MultiPolygon,
    GeometryType::GeometryCollection,
];

pub const SHAPEFILE_GEOMETRY_TYPES: &[GeometryType] = &[
    GeometryType::Point,
    GeometryType::LineString,
    GeometryType::Polygon,
    GeometryType::MultiPoint,
    GeometryType::MultiLineString,
    GeometryType::MultiPolygon,
];

pub const WKB_XY_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    supported: true,
    encodings: &[GeometryEncoding::Wkb],
    dimensions: &[CoordinateDimensions::Xy],
    spatial_semantics: &[SpatialSemantics::Geometry],
    geometry_types: SIMPLE_WKB_GEOMETRY_TYPES,
    mixed_types: true,
};

/// GeoJSON/KML-like geometry support: XY plus an interoperable altitude Z.
/// M is deliberately excluded because these formats do not assign it a stable
/// round-trip semantic.
pub const WKB_XY_XYZ_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    supported: true,
    encodings: &[GeometryEncoding::Wkb],
    dimensions: &[
        CoordinateDimensions::Xy,
        CoordinateDimensions::Xyz,
        CoordinateDimensions::Unknown,
    ],
    spatial_semantics: &[SpatialSemantics::Geometry],
    geometry_types: SIMPLE_WKB_GEOMETRY_TYPES,
    mixed_types: true,
};

pub const WKB_SINGLE_TYPE_XY_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    mixed_types: false,
    ..WKB_XY_GEOMETRY
};

/// Shapefile-like support: one native shape family per dataset, with the
/// dimensional variants represented by the format itself.
pub const WKB_SINGLE_TYPE_ALL_DIMENSIONS_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    geometry_types: SHAPEFILE_GEOMETRY_TYPES,
    mixed_types: false,
    ..WKB_PASSTHROUGH_GEOMETRY
};

pub const WKB_PASSTHROUGH_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    supported: true,
    encodings: &[GeometryEncoding::Wkb],
    dimensions: &[
        CoordinateDimensions::Xy,
        CoordinateDimensions::Xyz,
        CoordinateDimensions::Xym,
        CoordinateDimensions::Xyzm,
        CoordinateDimensions::Unknown,
    ],
    spatial_semantics: &[SpatialSemantics::Geometry],
    geometry_types: SIMPLE_WKB_GEOMETRY_TYPES,
    mixed_types: true,
};

pub const WKB_EWKB_PASSTHROUGH_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    encodings: &[GeometryEncoding::Wkb, GeometryEncoding::Ewkb],
    spatial_semantics: &[SpatialSemantics::Geometry, SpatialSemantics::Geography],
    ..WKB_PASSTHROUGH_GEOMETRY
};

pub const NO_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    supported: false,
    encodings: &[],
    dimensions: &[],
    spatial_semantics: &[],
    geometry_types: &[],
    mixed_types: false,
};

/// Garanzia offerta dal reader per `ReadRequest::projected_fields` (ADR-IO 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionSupport {
    /// La projection può essere ignorata in modalità `BestEffort`.
    None,
    /// Il reader può produrre esattamente e soltanto i campi richiesti.
    Exact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicatePruningSupport {
    None,
    NumericMinMaxStatistics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpatialPruningSupport {
    None,
    BoundingBoxStatistics,
    OptionalRtreeIndex,
}

#[derive(Clone, Debug, Serialize)]
pub struct FormatDescriptor {
    pub id: &'static str,
    pub direction: Direction,
    pub read_mode: ReadMode,
    /// Garanzia dell'operazione di lettura sul medesimo snapshot locale.
    pub read_determinism: DeterminismLevel,
    pub write_mode: Option<WriteMode>,
    /// Garanzia dell'operazione di scrittura; `None` per i driver read-only.
    pub write_determinism: Option<DeterminismLevel>,
    pub multi_layer: bool,
    pub multi_file: bool,
    /// Concorrenza dei reader ammessa dal formato (ADR-IO 1).
    pub reader_concurrency: ReaderConcurrency,
    /// Garanzia di projection applicabile al `ReadRequest` (ADR-IO 6).
    pub projection_support: ProjectionSupport,
    /// Pruning attributivo disponibile senza filtering riga-per-riga.
    pub predicate_pruning_support: PredicatePruningSupport,
    /// Pruning spaziale nativo; può dipendere dal dataset aperto.
    pub spatial_pruning_support: SpatialPruningSupport,
    pub crs_handling: CrsHandling,
    /// Capacità generale di fedeltà; la valutazione per-contratto è in open/create.
    pub fidelity_class: Fidelity,
    pub runtime: Runtime,
    pub write_capabilities: Option<FormatWriteCapabilities>,
    // Versioni esplicite: il fingerprint del catalogo deriva da queste (D17).
    pub semantic_version: u32,
    pub driver_version: u32,
    pub descriptor_version: u32,
}
