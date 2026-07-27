//! plenora-io-core — il confine plug-in dei driver di formato + macchinari
//! condivisi (descrittore, ReadRequest/WritePlan, registro, publish atomico,
//! LossReport). Nessun parser di formato qui: solo i contratti.
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
    ArrowTypeClass, AttributeWriteSupport, CrsHandling, CrsWriteSupport, Direction, Fidelity,
    FieldNamePolicy, FormatDescriptor, FormatWriteCapabilities, GeometryWriteSupport,
    NameNormalization, NullabilitySupport, ReadMode, ReaderConcurrency, Runtime, TextEncoding,
    TypeCoercionPolicy, WriteMode, ALL_ARROW_TYPES, DBF_FIELD_NAMES, NO_GEOMETRY, SCALAR_TYPES,
    UTF8_FIELD_NAMES, WKB_EWKB_PASSTHROUGH_GEOMETRY, WKB_PASSTHROUGH_GEOMETRY,
    WKB_SINGLE_TYPE_ALL_DIMENSIONS_GEOMETRY, WKB_SINGLE_TYPE_XY_GEOMETRY, WKB_XY_GEOMETRY,
    WKB_XY_XYZ_GEOMETRY,
};
pub use driver::{
    with_write_limits, with_write_validation, FormatDriver, FormatWriter, LayerReader,
    OpenDatasetHandle, Published, ReadOptions, Sink, Source, WriteOptions,
};
pub use loss::{
    FidelityAssessment, FidelityReason, FidelityReasonCode, LossExample, LossReport,
    MAX_FIDELITY_REASONS,
};
pub use publish::{
    publish_dir_atomic, publish_file_atomic, publish_file_atomic_limited,
    publish_files_ordered_limited, PublishOutcome,
};
pub use registry::DriverRegistry;
pub use request::{
    BatchTarget, Bbox, ProjectionMode, PruningPredicate, ReadRequest, WriteLayer, WritePlan,
};
