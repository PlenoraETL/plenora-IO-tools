//! Adattatore fail-closed tra parser push in background e `LayerReader`.
//!
//! Il protocollo ha un solo stato dati (`Batch`) e due stati terminali
//! espliciti (`Finished`, `Failed`). La chiusura del canale senza uno stato
//! terminale è sempre un errore: non può essere confusa con EOF.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::thread::JoinHandle;

use arrow_array::RecordBatch;
use plenora_io_model::contract::LayerContract;
use plenora_io_model::{PlenoraIoError, Result};

use super::LayerReader;

enum BatchWorkerEvent {
    Batch(RecordBatch),
    Finished,
    Failed(PlenoraIoError),
}

/// Unico canale con cui un parser in background può consegnare batch al core.
///
/// `send` restituisce `false` quando il reader è stato rilasciato: il producer
/// deve allora interrompere il lavoro senza trattare la cancellazione come un
/// errore del dataset.
pub struct BatchEmitter {
    sender: SyncSender<BatchWorkerEvent>,
}

impl BatchEmitter {
    pub fn send(&self, batch: RecordBatch) -> bool {
        self.sender.send(BatchWorkerEvent::Batch(batch)).is_ok()
    }
}

struct BatchWorkerReader {
    driver: &'static str,
    layer: LayerContract,
    receiver: Receiver<BatchWorkerEvent>,
    worker: Option<JoinHandle<()>>,
    terminal: bool,
}

impl BatchWorkerReader {
    fn abnormal_termination(&self) -> PlenoraIoError {
        PlenoraIoError::format(
            self.driver,
            "worker di lettura terminato senza stato terminale",
        )
    }

    fn join_worker(&mut self) -> Result<()> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker.join().map_err(|_| {
            PlenoraIoError::format(self.driver, "worker di lettura terminato in modo anomalo")
        })
    }
}

impl LayerReader for BatchWorkerReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if self.terminal {
            return Ok(None);
        }
        match self.receiver.recv() {
            Ok(BatchWorkerEvent::Batch(batch)) => Ok(Some(batch)),
            Ok(BatchWorkerEvent::Finished) => {
                self.terminal = true;
                self.join_worker()?;
                Ok(None)
            }
            Ok(BatchWorkerEvent::Failed(error)) => {
                self.terminal = true;
                self.join_worker()?;
                Err(error)
            }
            Err(_) => {
                self.terminal = true;
                let error = self.abnormal_termination();
                self.join_worker()?;
                Err(error)
            }
        }
    }
}

/// Avvia un parser bounded in background e ne espone l'output come
/// `LayerReader`.
///
/// Il wrapper emette sempre uno stato terminale esplicito. Gli errori conservano
/// la variante `PlenoraIoError`; un panic viene intercettato al confine del
/// thread e trasformato in errore di formato, mai in un falso EOF.
pub fn spawn_batch_reader<F>(
    driver: &'static str,
    layer: LayerContract,
    channel_capacity: usize,
    run: F,
) -> Result<Box<dyn LayerReader>>
where
    F: FnOnce(BatchEmitter) -> Result<()> + Send + 'static,
{
    let (sender, receiver) = sync_channel(channel_capacity);
    let terminal_sender = sender.clone();
    let worker = std::thread::Builder::new()
        .name(format!("plenora-{driver}-reader"))
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| run(BatchEmitter { sender })));
            let event = match result {
                Ok(Ok(())) => BatchWorkerEvent::Finished,
                Ok(Err(error)) => BatchWorkerEvent::Failed(error),
                Err(_) => BatchWorkerEvent::Failed(PlenoraIoError::format(
                    driver,
                    "worker di lettura terminato in modo anomalo",
                )),
            };
            drop(terminal_sender.send(event));
        })?;
    Ok(Box::new(BatchWorkerReader {
        driver,
        layer,
        receiver,
        worker: Some(worker),
        terminal: false,
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::RecordBatch;
    use arrow_schema::Schema;
    use plenora_io_model::contract::{DataContract, LayerContract, LayerId};

    use super::*;

    fn test_layer() -> LayerContract {
        LayerContract {
            id: LayerId(0),
            name: "layer".to_owned(),
            contract: DataContract {
                schema: Arc::new(Schema::empty()),
                geometry: None,
            },
        }
    }

    #[test]
    fn requires_explicit_successful_completion() {
        let mut reader = spawn_batch_reader("test", test_layer(), 1, |emitter| {
            assert!(emitter.send(RecordBatch::new_empty(Arc::new(Schema::empty()))));
            Ok(())
        })
        .unwrap();

        assert!(reader.next_batch().unwrap().is_some());
        assert!(reader.next_batch().unwrap().is_none());
        assert!(reader.next_batch().unwrap().is_none());
    }

    #[test]
    fn preserves_typed_errors() {
        let mut reader = spawn_batch_reader("test", test_layer(), 1, |_| {
            Err(PlenoraIoError::LimitExceeded(
                "limite del parser".to_owned(),
            ))
        })
        .unwrap();

        assert!(matches!(
            reader.next_batch(),
            Err(error)
                if error.code == plenora_io_model::IoErrorCode::LimitExceeded
                    && error.message == "limite del parser"
        ));
        assert!(reader.next_batch().unwrap().is_none());
    }

    #[test]
    fn turns_panic_into_error_instead_of_eof() {
        let mut reader = spawn_batch_reader("test", test_layer(), 1, |_| {
            panic!("panic intenzionale del test");
        })
        .unwrap();

        assert!(matches!(
            reader.next_batch(),
            Err(error)
                if error.code == plenora_io_model::IoErrorCode::Format
                    && error.driver.as_deref() == Some("test")
                    && error.message.contains("anomalo")
        ));
        assert!(reader.next_batch().unwrap().is_none());
    }
}
