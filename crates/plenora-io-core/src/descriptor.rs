//! `FormatDescriptor` — catalogo machine-readable dei driver (Architetture §2.3).

use serde::Serialize;

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

#[derive(Clone, Debug, Serialize)]
pub struct FormatDescriptor {
    pub id: &'static str,
    pub direction: Direction,
    pub read_mode: ReadMode,
    pub write_mode: Option<WriteMode>,
    pub multi_layer: bool,
    pub multi_file: bool,
    /// Concorrenza dei reader ammessa dal formato (ADR-IO 1).
    pub reader_concurrency: ReaderConcurrency,
    pub crs_handling: CrsHandling,
    /// Capacità generale di fedeltà; la valutazione per-contratto è in open/create.
    pub fidelity_class: Fidelity,
    pub runtime: Runtime,
    // Versioni esplicite: il fingerprint del catalogo deriva da queste (D17).
    pub semantic_version: u32,
    pub driver_version: u32,
    pub descriptor_version: u32,
}
