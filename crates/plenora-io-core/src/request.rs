//! `ReadRequest` (projection + pruning, mai filtering — ADR-IO 6) e `WritePlan`
//! (ADR-IO 1).

use arrow_schema::{DataType, Schema};
use plenora_core::contract::{DataContract, FieldId, LayerId};
use plenora_core::geometry::is_geometry_field;
use plenora_core::{PlenoraError, Result};

use crate::descriptor::{FormatDescriptor, ProjectionSupport};

#[derive(Clone, Copy, Debug)]
pub struct Bbox {
    pub minx: f64,
    pub miny: f64,
    pub maxx: f64,
    pub maxy: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PruningComparison {
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Equal,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PruningScalar {
    Int64(i64),
    Float64(f64),
}

/// Suggerimento di pruning interpretato dal driver **solo** se ha una capacità
/// nativa equivalente (es. min/max di row group). Non è un filtro:
/// over-return ammesso, under-return vietato (ADR-IO 6).
#[derive(Clone, Debug)]
pub enum PruningPredicate {
    NumericComparison {
        field: FieldId,
        comparison: PruningComparison,
        value: PruningScalar,
    },
    /// Compatibilità v1. Le espressioni non riconosciute vengono ignorate.
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

/// Applica la semantica fail-closed di `ProjectionMode::Required`.
pub fn validate_read_projection(
    descriptor: &FormatDescriptor,
    request: &ReadRequest,
) -> Result<()> {
    if request.projection_mode == ProjectionMode::Required
        && request.projected_fields.is_some()
        && descriptor.projection_support != ProjectionSupport::Exact
    {
        return Err(PlenoraError::ProjectionUnsupported {
            driver: descriptor.id,
        });
    }
    Ok(())
}

/// Traduce il target in byte in un numero di righe conservativo per i reader
/// che costruiscono batch incrementali. È una stima (ADR-IO 6), non un limite
/// di memoria: le colonne variabili e la geometria possono avere righe atipiche.
pub fn effective_batch_rows(schema: &Schema, target: BatchTarget) -> usize {
    let estimated_row_bytes = schema
        .fields()
        .iter()
        .map(|field| {
            if is_geometry_field(field) {
                512
            } else {
                estimated_type_bytes(field.data_type())
            }
        })
        .sum::<usize>()
        .max(1);
    let byte_rows = target.target_bytes.max(1) / estimated_row_bytes;
    target.max_rows.max(1).min(byte_rows.max(1))
}

fn estimated_type_bytes(data_type: &DataType) -> usize {
    match data_type {
        DataType::Boolean | DataType::Int8 | DataType::UInt8 => 1,
        DataType::Int16 | DataType::UInt16 | DataType::Float16 => 2,
        DataType::Int32
        | DataType::UInt32
        | DataType::Float32
        | DataType::Date32
        | DataType::Time32(_) => 4,
        DataType::Int64
        | DataType::UInt64
        | DataType::Float64
        | DataType::Date64
        | DataType::Time64(_)
        | DataType::Timestamp(_, _)
        | DataType::Duration(_) => 8,
        DataType::Decimal128(_, _) => 16,
        DataType::Decimal256(_, _) => 32,
        DataType::FixedSizeBinary(size) => (*size).max(1) as usize,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => 64,
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView => 128,
        _ => 64,
    }
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use arrow_schema::{DataType, Field, Schema};

    use super::*;

    #[test]
    fn target_bytes_reduces_rows_but_never_below_one() {
        let schema = Schema::new(vec![
            Field::new("name", DataType::Utf8, true),
            Field::new("value", DataType::Int64, false),
        ]);
        assert_eq!(
            effective_batch_rows(
                &schema,
                BatchTarget {
                    target_bytes: 720,
                    max_rows: 100,
                },
            ),
            10
        );
        assert_eq!(
            effective_batch_rows(
                &schema,
                BatchTarget {
                    target_bytes: 0,
                    max_rows: 0,
                },
            ),
            1
        );
    }

    #[test]
    fn geometry_uses_a_conservative_variable_width_estimate() {
        let field =
            Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([(
                plenora_core::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
                plenora_core::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
            )]));
        let schema = Schema::new(vec![field]);
        assert_eq!(
            effective_batch_rows(
                &schema,
                BatchTarget {
                    target_bytes: 1024,
                    max_rows: 100,
                },
            ),
            2
        );
    }
}
