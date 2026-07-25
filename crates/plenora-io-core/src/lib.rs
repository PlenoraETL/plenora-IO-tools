//! plenora-io-core — il confine plug-in dei driver di formato + macchinari
//! condivisi (descrittore, ReadRequest/WritePlan, registro, publish atomico,
//! LossReport). Nessun parser di formato qui: solo i contratti.
#![forbid(unsafe_code)]

pub mod descriptor;
pub mod driver;
pub mod loss;
pub mod publish;
pub mod registry;
pub mod request;

pub use descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, ReadMode, ReaderConcurrency, Runtime,
    WriteMode,
};
pub use driver::{
    FormatDriver, FormatWriter, LayerReader, OpenDatasetHandle, Published, ReadOptions, Sink,
    Source, WriteOptions,
};
pub use loss::{LossExample, LossReport};
pub use publish::{publish_dir_atomic, publish_file_atomic, PublishOutcome};
pub use registry::DriverRegistry;
pub use request::{
    BatchTarget, Bbox, ProjectionMode, PruningPredicate, ReadRequest, WriteLayer, WritePlan,
};
