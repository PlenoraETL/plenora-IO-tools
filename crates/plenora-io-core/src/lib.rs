//! plenora-io-core — il confine plug-in dei driver di formato + macchinari
//! condivisi (descrittore, `ReadRequest`/`WritePlan`, registro, publish
//! atomico, `LossReport`). Nessun parser di formato qui: solo i contratti.
#![forbid(unsafe_code)]

pub mod capabilities;
pub mod descriptor;
pub mod driver;
pub mod loss;
pub mod publish;
pub mod registry;
pub mod request;

pub use capabilities::{arrow_type_class, validate_write};
pub use descriptor::{
    ArrowTypeClass, AttributeWriteSupport, CrsHandling, CrsRepresentationCapabilities,
    CrsRepresentationState, CrsWriteSupport, DeterminismLevel, Direction, Fidelity,
    FieldNamePolicy, FormatDescriptor, FormatWriteCapabilities, GeometryWriteSupport,
    NameNormalization, NullabilitySupport, PredicatePruningSupport, ProjectionSupport, ReadMode,
    ReaderConcurrency, Runtime, SpatialPruningSupport, TextEncoding, TypeCoercionPolicy, WriteMode,
    ALL_ARROW_TYPES, ALL_GEOMETRY_TYPES, DBF_FIELD_NAMES, NO_GEOMETRY, SCALAR_TYPES,
    SHAPEFILE_GEOMETRY_TYPES, SIMPLE_WKB_GEOMETRY_TYPES, UTF8_FIELD_NAMES,
    WKB_EWKB_PASSTHROUGH_GEOMETRY, WKB_PASSTHROUGH_GEOMETRY,
    WKB_SINGLE_TYPE_ALL_DIMENSIONS_GEOMETRY, WKB_SINGLE_TYPE_XY_GEOMETRY, WKB_XY_GEOMETRY,
    WKB_XY_XYZ_GEOMETRY,
};
pub use driver::{
    check_cancelled, check_cancelled_periodically, read_row_error, spawn_batch_reader,
    with_batch_target, with_cancellation, with_read_budget, with_write_limits,
    with_write_validation, write_row_rejection, BatchEmitter, FormatDriver, FormatWriter,
    LayerReader, OpenDatasetHandle, Published, ReadOptions, SingleReaderGate, Sink, Source,
    WriteOptions, CANCELLATION_CHECK_INTERVAL,
};
pub use loss::{
    FidelityAssessment, FidelityReason, FidelityReasonCode, LossExample, LossReport,
    INCONSISTENT_CRS_REPRESENTATIONS, MAX_FIDELITY_REASONS,
};
pub use publish::{
    create_staged_dir, create_staged_file, create_staged_file_with_suffix, publish_dir_atomic,
    publish_file_atomic, publish_file_atomic_limited, publish_files_ordered_limited,
    PublishOutcome, StagedFile,
};
pub use registry::DriverRegistry;
pub use request::{
    effective_batch_rows, incremental_batch_memory_size, project_layer_contract,
    validate_read_projection, AdaptiveBatchSizer, BatchTarget, Bbox, ProjectionMode,
    PruningComparison, PruningPredicate, PruningScalar, ReadRequest, ReadScope, WriteLayer,
    WritePlan,
};
