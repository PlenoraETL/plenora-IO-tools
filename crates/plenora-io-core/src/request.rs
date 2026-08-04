//! `ReadRequest` (projection + pruning, mai filtering — ADR-IO 6) e `WritePlan`
//! (ADR-IO 1).

use std::sync::Arc;

use arrow_array::{Array, RecordBatch};
use arrow_schema::{DataType, Schema};
use plenora_io_model::contract::{
    DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId,
};
use plenora_io_model::geometry::is_geometry_field;
use plenora_io_model::CancellationToken;
use plenora_io_model::{PlenoraIoError, Result};

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

/// Estensione della conoscenza richiesta al reader comune.
///
/// `Complete` valida l'operazione fino a EOF ed e' obbligatorio per pipeline
/// operation-atomic come `convert`. `AcceptedRows` consente a un consumatore di
/// summary di arrestarsi dopo il primo batch che porta almeno alla soglia; il
/// batch non viene affettato, quindi resta valido l'overshoot storico. Una
/// diagnostica prodotta prima dello stop e' necessariamente non completa.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReadScope {
    #[default]
    Complete,
    AcceptedRows(u64),
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

#[derive(Clone, Copy, Debug)]
pub struct AdaptiveBatchSizer {
    target: BatchTarget,
    rows: usize,
}

impl AdaptiveBatchSizer {
    pub fn new(schema: &Schema, target: BatchTarget) -> Self {
        Self {
            target,
            rows: effective_batch_rows(schema, target),
        }
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn observe(&mut self, batch: &RecordBatch) {
        self.rows = observed_batch_rows(
            batch.num_rows(),
            incremental_batch_memory_size(batch),
            self.target,
        );
    }
}

/// Memoria attribuibile al batch corrente: metadata Arrow posseduti e porzione
/// logica dei buffer necessaria alla slice. Evita di riaddebitare l'intera
/// capacity di un parent condiviso, ma continua a contare integralmente un
/// batch non affettato e quindi realmente grande.
pub fn incremental_batch_memory_size(batch: &RecordBatch) -> usize {
    batch.columns().iter().fold(0usize, |total, array| {
        let data = array.to_data();
        let metadata = data
            .get_array_memory_size()
            .saturating_sub(data.get_buffer_memory_size());
        let attributed = data.get_slice_memory_size().map_or_else(
            |_| data.get_array_memory_size(),
            |slice| metadata.saturating_add(slice),
        );
        total.saturating_add(attributed)
    })
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
    pub scope: ReadScope,
    pub batch_target: BatchTarget,
    pub cancellation: CancellationToken,
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
        return Err(PlenoraIoError::projection_unsupported(descriptor.id));
    }
    Ok(())
}

/// Risolve una projection esatta e costruisce il contratto effettivo.
///
/// Gli indici restituiti sono deduplicati e ordinati secondo lo schema nativo.
/// In `BestEffort` gli ID fuori range vengono ignorati; in `Required` causano
/// un errore all'apertura del reader.
pub fn project_layer_contract(
    source: &LayerContract,
    request: &ReadRequest,
) -> Result<(Vec<usize>, LayerContract)> {
    let mut indices = match &request.projected_fields {
        None => (0..source.contract.schema.fields().len()).collect::<Vec<_>>(),
        Some(field_ids) => {
            let mut indices = Vec::with_capacity(field_ids.len());
            for field_id in field_ids {
                let index = field_id.0 as usize;
                if index >= source.contract.schema.fields().len() {
                    if request.projection_mode == ProjectionMode::Required {
                        return Err(PlenoraIoError::Contract(format!(
                            "projection Required: field id {} fuori range",
                            field_id.0
                        )));
                    }
                    continue;
                }
                if !indices.contains(&index) {
                    indices.push(index);
                }
            }
            indices
        }
    };
    indices.sort_unstable();

    let fields = indices
        .iter()
        .map(|&index| source.contract.schema.field(index).as_ref().clone())
        .collect::<Vec<_>>();
    let schema = Arc::new(Schema::new_with_metadata(
        fields,
        source.contract.schema.metadata().clone(),
    ));
    let geometry = source.contract.geometry.clone().and_then(|geometry| {
        indices
            .iter()
            .position(|&index| index == geometry.field_id.0 as usize)
            .map(|index| GeometryColumnContract {
                field_id: FieldId(index as u32),
                ..geometry
            })
    });
    let mut layer = source.clone();
    layer.contract = DataContract { schema, geometry };
    Ok((indices, layer))
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

fn observed_batch_rows(row_count: usize, batch_bytes: usize, target: BatchTarget) -> usize {
    if row_count == 0 || batch_bytes == 0 {
        return target.max_rows.max(1);
    }
    let rows_for_bytes =
        (target.target_bytes.max(1).saturating_mul(row_count) / batch_bytes).max(1);
    rows_for_bytes.min(target.max_rows.max(1))
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
    use plenora_io_model::crs::CrsResolution;

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
                plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
                plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
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

    #[test]
    fn observed_batch_sizing_respects_both_limits() {
        let target = BatchTarget {
            target_bytes: 1_000,
            max_rows: 100,
        };
        assert_eq!(observed_batch_rows(20, 4_000, target), 5);
        assert_eq!(observed_batch_rows(20, 10, target), 100);
        assert_eq!(observed_batch_rows(20, usize::MAX, target), 1);
        assert_eq!(observed_batch_rows(0, 0, target), 100);
    }

    #[test]
    fn exact_projection_is_deduplicated_ordered_and_fail_closed() {
        let source = LayerContract {
            id: LayerId(7),
            name: "source".to_owned(),
            contract: DataContract::new(
                Arc::new(Schema::new(vec![
                    Field::new("a", DataType::Int64, false),
                    Field::new("b", DataType::Utf8, true),
                    Field::new("c", DataType::Float64, false),
                ])),
                None,
            ),
        };
        let request = ReadRequest {
            layer: source.id,
            projected_fields: Some(vec![FieldId(2), FieldId(0), FieldId(2)]),
            projection_mode: ProjectionMode::Required,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::Complete,
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        };
        let (indices, projected) = match project_layer_contract(&source, &request) {
            Ok(projected) => projected,
            Err(error) => panic!("projection valida rifiutata: {error}"),
        };
        assert_eq!(indices, vec![0, 2]);
        assert_eq!(projected.contract.schema.field(0).name(), "a");
        assert_eq!(projected.contract.schema.field(1).name(), "c");

        let invalid = ReadRequest {
            projected_fields: Some(vec![FieldId(3)]),
            ..request
        };
        assert!(project_layer_contract(&source, &invalid).is_err());

        let geometry_source = LayerContract {
            contract: DataContract::new(
                Arc::new(Schema::new(vec![
                    Field::new("a", DataType::Int64, false),
                    Field::new("geometry", DataType::Binary, true),
                    Field::new("c", DataType::Float64, false),
                ])),
                Some(GeometryColumnContract::wkb_xy(
                    FieldId(1),
                    "geometry",
                    CrsResolution::Missing,
                    true,
                )),
            ),
            ..source
        };
        let geometry_request = ReadRequest {
            layer: geometry_source.id,
            projected_fields: Some(vec![FieldId(2), FieldId(1)]),
            projection_mode: ProjectionMode::Required,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::Complete,
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        };
        let projected = match project_layer_contract(&geometry_source, &geometry_request) {
            Ok((_, projected)) => projected,
            Err(error) => panic!("projection geometrica valida rifiutata: {error}"),
        };
        assert_eq!(
            projected
                .contract
                .geometry
                .map(|geometry| geometry.field_id),
            Some(FieldId(0))
        );
    }
}
