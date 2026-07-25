//! `ReadRequest` (projection + pruning, mai filtering — ADR-IO 6) e `WritePlan`
//! (ADR-IO 1).

use plenora_core::contract::{DataContract, FieldId, LayerId};

#[derive(Clone, Copy, Debug)]
pub struct Bbox {
    pub minx: f64,
    pub miny: f64,
    pub maxx: f64,
    pub maxy: f64,
}

/// Suggerimento di pruning: opaco nella v1, interpretato dal driver **solo** se
/// ha una capacità nativa equivalente (min/max row group, indice spaziale).
/// Non è un filtro: over-return ammesso, under-return vietato (ADR-IO 6).
#[derive(Clone, Debug)]
pub enum PruningPredicate {
    Opaque(String),
}

/// Modalità di projection (ADR-IO 6). `Required` = il reader produce esattamente
/// la projection o fallisce all'apertura; `BestEffort` = può restituire colonne
/// extra, lo schema effettivo del reader resta autoritativo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionMode {
    Required,
    BestEffort,
}

#[derive(Clone, Copy, Debug)]
pub struct BatchTarget {
    pub target_bytes: usize,
    pub max_rows: usize,
}

impl Default for BatchTarget {
    fn default() -> Self {
        Self {
            target_bytes: 8 * 1024 * 1024,
            max_rows: 65_536,
        }
    }
}

pub struct ReadRequest {
    pub layer: LayerId,
    /// La geometria è inclusa solo se richiesta dalla projection, necessaria per
    /// `spatial_pruning_hint`, o richiesta dal contratto del consumatore — mai
    /// forzata per letture puramente tabellari (ADR-IO 6).
    pub projected_fields: Option<Vec<FieldId>>,
    pub projection_mode: ProjectionMode,
    pub pruning_predicate: Option<PruningPredicate>,
    pub spatial_pruning_hint: Option<Bbox>,
    pub batch_target: BatchTarget,
}

/// Piano di scrittura di un dataset: 1..N layer pubblicati insieme (D11).
/// L'ordine è canonico; i nomi devono essere unici (validato dal driver).
pub struct WritePlan {
    pub layers: Vec<WriteLayer>,
}

pub struct WriteLayer {
    pub name: String,
    pub contract: DataContract,
}
