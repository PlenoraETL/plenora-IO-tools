//! Adattatori comuni applicati ai `LayerReader`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use arrow_array::{Array, BinaryArray, LargeBinaryArray, RecordBatch};
use plenora_io_model::contract::{LayerContract, LayerId};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::resource::{ResourceBudget, ResourceKind, ResourceLease};
use plenora_io_model::wkb::inspect_wkb;
use plenora_io_model::{CancellationToken, ErrorPhase, PlenoraIoError, Result};

use crate::loss::{declare_crs_inconsistency, LossReport};
use crate::request::{effective_batch_rows, BatchTarget};

use super::{saturating_usize, LayerReader, OpenDatasetHandle};

/// Collega a un dataset un budget condiviso. Ogni reader consuma colonne,
/// righe, byte e una quota di concorrenza dagli stessi contatori, anche quando
/// il budget attraversa più componenti della pipeline.
pub fn with_read_budget(
    dataset: Box<dyn OpenDatasetHandle>,
    budget: ResourceBudget,
) -> Box<dyn OpenDatasetHandle> {
    Box::new(BudgetedDataset { dataset, budget })
}

struct BudgetedDataset {
    dataset: Box<dyn OpenDatasetHandle>,
    budget: ResourceBudget,
}

impl OpenDatasetHandle for BudgetedDataset {
    fn layers(&self) -> &[LayerContract] {
        self.dataset.layers()
    }

    fn fidelity_assessment(&self) -> crate::loss::FidelityAssessment {
        self.dataset.fidelity_assessment()
    }

    fn open_layer_reader(
        &self,
        request: &crate::request::ReadRequest,
    ) -> Result<Box<dyn LayerReader>> {
        self.budget.ensure_active()?;
        let reader = self.dataset.open_layer_reader(request)?;
        BudgetedReader::new(reader, self.budget.clone())
            .map(|reader| Box::new(reader) as Box<dyn LayerReader>)
    }
}

struct BudgetedReader {
    inner: Box<dyn LayerReader>,
    budget: ResourceBudget,
    _operation_lease: ResourceLease,
}

impl BudgetedReader {
    fn new(inner: Box<dyn LayerReader>, budget: ResourceBudget) -> Result<Self> {
        let operation_lease = budget.try_lease(ResourceKind::ConcurrentOperations, 1)?;
        let columns = u64::try_from(inner.contract().contract.schema.fields().len())
            .map_err(|_| PlenoraIoError::LimitExceeded("troppe colonne nel reader".to_owned()))?;
        if columns > 0 {
            budget
                .try_lease(ResourceKind::Columns, columns)?
                .commit(columns)?;
        }
        Ok(Self {
            inner,
            budget,
            _operation_lease: operation_lease,
        })
    }
}

impl LayerReader for BudgetedReader {
    fn contract(&self) -> &LayerContract {
        self.inner.contract()
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        self.budget.ensure_active()?;
        let Some(batch) = self.inner.next_batch()? else {
            return Ok(None);
        };
        let rows = u64::try_from(batch.num_rows()).map_err(|_| {
            PlenoraIoError::LimitExceeded("batch oltre il conteggio supportato".to_owned())
        })?;
        if rows == 0 {
            return Ok(Some(batch));
        }
        let bytes = u64::try_from(batch.get_array_memory_size()).map_err(|_| {
            PlenoraIoError::LimitExceeded("batch oltre il conteggio byte supportato".to_owned())
        })?;
        let rows_lease = self.budget.try_lease(ResourceKind::Rows, rows)?;
        let output_lease = (bytes > 0)
            .then(|| self.budget.try_lease(ResourceKind::OutputBytes, bytes))
            .transpose()?;
        let memory_lease = (bytes > 0)
            .then(|| self.budget.try_lease(ResourceKind::MemoryBytes, bytes))
            .transpose()?;
        let geometry_components = geometry_components(self.inner.contract(), &batch, &self.budget)?;
        let geometry_lease = (geometry_components > 0)
            .then(|| {
                self.budget
                    .try_lease(ResourceKind::GeometryComponents, geometry_components)
            })
            .transpose()?;
        rows_lease.commit(rows)?;
        if let Some(output_lease) = output_lease {
            output_lease.commit(bytes)?;
        }
        if let Some(memory_lease) = memory_lease {
            memory_lease.commit(bytes)?;
        }
        if let Some(geometry_lease) = geometry_lease {
            geometry_lease.commit(geometry_components)?;
        }
        Ok(Some(batch))
    }

    fn loss_report(&self) -> LossReport {
        reader_loss(self.inner.as_ref())
    }
}

fn geometry_components(
    contract: &LayerContract,
    batch: &RecordBatch,
    budget: &ResourceBudget,
) -> Result<u64> {
    let Some(geometry) = &contract.contract.geometry else {
        return Ok(0);
    };
    let Some(index) = batch
        .schema()
        .fields()
        .iter()
        .position(|field| field.name() == &geometry.name)
    else {
        // Una projection tabellare può escludere legittimamente la geometria.
        return Ok(0);
    };
    let limits = WkbLimits {
        max_cell_bytes: saturating_usize(budget.limits().cell_bytes),
        max_components: saturating_usize(budget.remaining(ResourceKind::GeometryComponents)),
        max_depth: saturating_usize(budget.limits().nesting_depth),
    };
    let array = batch.column(index);
    let mut total = 0_u64;
    let mut inspect = |bytes: &[u8]| -> Result<()> {
        let components = u64::try_from(inspect_wkb(bytes, &limits)?.components).map_err(|_| {
            PlenoraIoError::LimitExceeded("geometria oltre il conteggio supportato".to_owned())
        })?;
        total = total.checked_add(components).ok_or_else(|| {
            PlenoraIoError::LimitExceeded(
                "overflow nel conteggio dei componenti geometrici".to_owned(),
            )
        })?;
        Ok(())
    };
    if let Some(values) = array.as_any().downcast_ref::<BinaryArray>() {
        for row in 0..values.len() {
            if !values.is_null(row) {
                inspect(values.value(row))?;
            }
        }
        return Ok(total);
    }
    if let Some(values) = array.as_any().downcast_ref::<LargeBinaryArray>() {
        for row in 0..values.len() {
            if !values.is_null(row) {
                inspect(values.value(row))?;
            }
        }
        return Ok(total);
    }
    Err(PlenoraIoError::LimitExceeded(
        "colonna geometrica non binaria nel reader budgeted".to_owned(),
    ))
}

/// Adatta i batch prodotti da un reader al target comune di ADR-IO 6.
///
/// Lo slicing Arrow non copia i buffer e quindi limita la cardinalità esposta,
/// non la memoria già allocata dal reader sottostante.
pub fn with_batch_target(
    reader: Box<dyn LayerReader>,
    target: BatchTarget,
    cancellation: CancellationToken,
) -> Box<dyn LayerReader> {
    let rows_per_batch = effective_batch_rows(reader.contract().contract.schema.as_ref(), target);
    Box::new(BatchTargetReader {
        inner: reader,
        rows_per_batch,
        pending: None,
        cancellation,
    })
}

/// Collega il token R11 a un reader e rilascia immediatamente il reader
/// sottostante quando la cancellazione viene osservata.
pub fn with_cancellation(
    reader: Box<dyn LayerReader>,
    cancellation: CancellationToken,
) -> Box<dyn LayerReader> {
    let loss = reader_loss(reader.as_ref());
    Box::new(CancellationReader {
        contract: reader.contract().clone(),
        inner: Some(reader),
        loss,
        cancellation,
    })
}

struct CancellationReader {
    contract: LayerContract,
    inner: Option<Box<dyn LayerReader>>,
    loss: LossReport,
    cancellation: CancellationToken,
}

impl LayerReader for CancellationReader {
    fn contract(&self) -> &LayerContract {
        &self.contract
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if let Err(error) = super::check_cancelled(&self.cancellation, ErrorPhase::Read) {
            self.inner = None;
            return Err(error);
        }
        let Some(inner) = self.inner.as_mut() else {
            return Ok(None);
        };
        let result = inner.next_batch();
        self.loss = reader_loss(inner.as_ref());
        if !matches!(result, Ok(Some(_))) {
            self.inner = None;
        }
        result
    }

    fn loss_report(&self) -> LossReport {
        self.loss.clone()
    }
}

struct BatchTargetReader {
    inner: Box<dyn LayerReader>,
    rows_per_batch: usize,
    pending: Option<(RecordBatch, usize)>,
    cancellation: CancellationToken,
}

impl LayerReader for BatchTargetReader {
    fn contract(&self) -> &LayerContract {
        self.inner.contract()
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        super::check_cancelled(&self.cancellation, ErrorPhase::Read)?;
        loop {
            if let Some((batch, offset)) = self.pending.take() {
                let remaining = batch.num_rows() - offset;
                let take = remaining.min(self.rows_per_batch);
                let output = batch.slice(offset, take);
                if take < remaining {
                    self.pending = Some((batch, offset + take));
                }
                return Ok(Some(output));
            }

            let Some(batch) = self.inner.next_batch()? else {
                return Ok(None);
            };
            if batch.num_rows() <= self.rows_per_batch {
                return Ok(Some(batch));
            }
            self.pending = Some((batch, 0));
        }
    }

    fn loss_report(&self) -> LossReport {
        reader_loss(self.inner.as_ref())
    }
}

fn reader_loss(reader: &dyn LayerReader) -> LossReport {
    let mut loss = reader.loss_report();
    declare_crs_inconsistency(reader.contract(), &mut loss);
    loss
}

/// Enforcement runtime di `ReaderConcurrency::SingleActiveReader` (ADR-IO 1).
/// Il lease è per-handle: viene rilasciato a EOF/errore o al drop anticipato.
#[derive(Clone)]
pub struct SingleReaderGate {
    active: Arc<AtomicBool>,
    driver: &'static str,
}

impl SingleReaderGate {
    pub fn new(driver: &'static str) -> Self {
        Self {
            active: Arc::new(AtomicBool::new(false)),
            driver,
        }
    }

    pub fn open<F>(&self, layer: LayerId, create: F) -> Result<Box<dyn LayerReader>>
    where
        F: FnOnce() -> Result<Box<dyn LayerReader>>,
    {
        self.active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| PlenoraIoError::reader_busy(self.driver, layer.0))?;

        let lease = ReaderLease {
            active: self.active.clone(),
            released: false,
        };
        match create() {
            Ok(inner) => Ok(Box::new(SingleActiveLayerReader { inner, lease })),
            Err(error) => {
                drop(lease);
                Err(error)
            }
        }
    }
}

struct ReaderLease {
    active: Arc<AtomicBool>,
    released: bool,
}

impl ReaderLease {
    fn release(&mut self) {
        if !self.released {
            self.active.store(false, Ordering::Release);
            self.released = true;
        }
    }
}

impl Drop for ReaderLease {
    fn drop(&mut self) {
        self.release();
    }
}

struct SingleActiveLayerReader {
    inner: Box<dyn LayerReader>,
    lease: ReaderLease,
}

impl LayerReader for SingleActiveLayerReader {
    fn contract(&self) -> &LayerContract {
        self.inner.contract()
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        let result = self.inner.next_batch();
        if !matches!(result, Ok(Some(_))) {
            self.lease.release();
        }
        result
    }

    fn loss_report(&self) -> LossReport {
        self.inner.loss_report()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::Int64Array;
    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_model::contract::{DataContract, LayerId};
    use plenora_io_model::resource::ResourceLimits;

    use super::*;

    struct OneBatchReader {
        contract: LayerContract,
        batch: Option<RecordBatch>,
    }

    impl OneBatchReader {
        fn new(values: Vec<i64>) -> Self {
            let schema = Arc::new(Schema::new(vec![Field::new(
                "value",
                DataType::Int64,
                false,
            )]));
            let batch =
                RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(values))])
                    .unwrap();
            Self {
                contract: LayerContract {
                    id: LayerId(0),
                    name: "values".to_owned(),
                    contract: DataContract {
                        schema,
                        geometry: None,
                    },
                },
                batch: Some(batch),
            }
        }
    }

    impl LayerReader for OneBatchReader {
        fn contract(&self) -> &LayerContract {
            &self.contract
        }

        fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            Ok(self.batch.take())
        }
    }

    #[test]
    fn row_quota_is_shared_across_independent_readers() {
        let budget = ResourceBudget::new(ResourceLimits {
            rows: 3,
            columns: 10,
            memory_bytes: 1024 * 1024,
            cell_bytes: 1024,
            output_bytes: 1024 * 1024,
            ..ResourceLimits::default()
        })
        .unwrap();
        let mut first =
            BudgetedReader::new(Box::new(OneBatchReader::new(vec![1, 2])), budget.clone()).unwrap();
        assert_eq!(first.next_batch().unwrap().unwrap().num_rows(), 2);
        drop(first);

        let mut second =
            BudgetedReader::new(Box::new(OneBatchReader::new(vec![3, 4])), budget).unwrap();
        let error = second.next_batch().unwrap_err();
        assert_eq!(
            error.category,
            plenora_io_model::ErrorCategory::ResourceLimit
        );
    }
}
