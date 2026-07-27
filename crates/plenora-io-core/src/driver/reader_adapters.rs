//! Adattatori comuni applicati ai `LayerReader`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use arrow_array::RecordBatch;
use plenora_io_model::contract::{LayerContract, LayerId};
use plenora_io_model::{PlenoraIoError, Result};

use crate::loss::LossReport;
use crate::request::{effective_batch_rows, BatchTarget};

use super::LayerReader;

/// Adatta i batch prodotti da un reader al target comune di ADR-IO 6.
///
/// Lo slicing Arrow non copia i buffer e quindi limita la cardinalità esposta,
/// non la memoria già allocata dal reader sottostante.
pub fn with_batch_target(
    reader: Box<dyn LayerReader>,
    target: BatchTarget,
) -> Box<dyn LayerReader> {
    let rows_per_batch = effective_batch_rows(reader.contract().contract.schema.as_ref(), target);
    Box::new(BatchTargetReader {
        inner: reader,
        rows_per_batch,
        pending: None,
    })
}

struct BatchTargetReader {
    inner: Box<dyn LayerReader>,
    rows_per_batch: usize,
    pending: Option<(RecordBatch, usize)>,
}

impl LayerReader for BatchTargetReader {
    fn contract(&self) -> &LayerContract {
        self.inner.contract()
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
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
        self.inner.loss_report()
    }
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
            .map_err(|_| PlenoraIoError::ReaderBusy {
                driver: self.driver,
                layer: layer.0,
            })?;

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
