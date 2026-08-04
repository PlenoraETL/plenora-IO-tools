//! Adattatore fail-closed tra parser push in background e `LayerReader`.
//!
//! Il protocollo ha un solo stato dati (`Batch`) e due stati terminali
//! espliciti (`Finished`, `Failed`). La chiusura del canale senza uno stato
//! terminale è sempre un errore: non può essere confusa con EOF.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::Duration;

use arrow_array::RecordBatch;
use plenora_io_model::contract::LayerContract;
use plenora_io_model::{CancellationToken, ErrorPhase, PlenoraIoError, Result};

use super::LayerReader;

enum BatchWorkerEvent {
    Batch(RecordBatch),
    Heartbeat,
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

    /// Consegna con backpressure senza rendere il producer cieco alla
    /// cancellazione mentre il canale bounded e' pieno.
    pub fn send_cancellable(
        &self,
        mut batch: RecordBatch,
        cancellation: &CancellationToken,
        phase: ErrorPhase,
    ) -> Result<bool> {
        const INITIAL_BACKOFF: Duration = Duration::from_micros(50);
        const MAX_BACKOFF: Duration = Duration::from_millis(5);
        let mut backoff = INITIAL_BACKOFF;
        loop {
            super::check_cancelled(cancellation, phase)?;
            match self.sender.try_send(BatchWorkerEvent::Batch(batch)) {
                Ok(()) => return Ok(true),
                Err(TrySendError::Disconnected(_)) => return Ok(false),
                Err(TrySendError::Full(event)) => match event {
                    BatchWorkerEvent::Batch(returned) => {
                        batch = returned;
                        std::thread::park_timeout(backoff);
                        backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
                    }
                    BatchWorkerEvent::Heartbeat
                    | BatchWorkerEvent::Finished
                    | BatchWorkerEvent::Failed(_) => {
                        return Err(PlenoraIoError::format(
                            "batch-worker",
                            "il canale bounded ha restituito un evento diverso dal batch inviato",
                        ));
                    }
                },
            }
        }
    }

    /// Controlla senza bloccare se il consumer esiste ancora.
    pub fn is_receiver_alive(&self) -> bool {
        match self.sender.try_send(BatchWorkerEvent::Heartbeat) {
            Ok(()) | Err(TrySendError::Full(_)) => true,
            Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

struct BatchWorkerReader {
    driver: &'static str,
    layer: LayerContract,
    receiver: Receiver<BatchWorkerEvent>,
    worker: Option<JoinHandle<()>>,
    terminal: bool,
    terminal_error: Option<PlenoraIoError>,
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
        if let Some(error) = &self.terminal_error {
            return Err(error.clone());
        }
        if self.terminal {
            return Ok(None);
        }
        loop {
            match self.receiver.recv() {
                Ok(BatchWorkerEvent::Batch(batch)) => return Ok(Some(batch)),
                Ok(BatchWorkerEvent::Heartbeat) => continue,
                Ok(BatchWorkerEvent::Finished) => {
                    self.terminal = true;
                    self.join_worker()?;
                    return Ok(None);
                }
                Ok(BatchWorkerEvent::Failed(error)) => {
                    self.terminal = true;
                    // L'errore tipizzato inviato dal producer è l'esito
                    // autorevole. Un eventuale panic del join non può
                    // sovrascriverne categoria, causa o diagnostica.
                    drop(self.join_worker());
                    self.terminal_error = Some(error.clone());
                    return Err(error);
                }
                Err(_) => {
                    self.terminal = true;
                    let error = self.abnormal_termination();
                    drop(self.join_worker());
                    self.terminal_error = Some(error.clone());
                    return Err(error);
                }
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
        terminal_error: None,
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
        assert!(matches!(
            reader.next_batch(),
            Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded
        ));
    }

    #[test]
    fn heartbeat_is_invisible_to_the_reader() {
        let mut reader = spawn_batch_reader("test", test_layer(), 2, |emitter| {
            assert!(emitter.is_receiver_alive());
            assert!(emitter.send(RecordBatch::new_empty(Arc::new(Schema::empty()))));
            Ok(())
        })
        .unwrap();

        assert!(reader.next_batch().unwrap().is_some());
        assert!(reader.next_batch().unwrap().is_none());
    }

    #[test]
    fn heartbeat_detects_a_dropped_reader() {
        let (sender, receiver) = sync_channel(1);
        let emitter = BatchEmitter { sender };
        drop(receiver);

        assert!(!emitter.is_receiver_alive());
    }

    #[test]
    fn cancellable_send_observes_pre_cancelled_token_on_a_full_channel() {
        let (sender, _receiver) = sync_channel(1);
        sender.send(BatchWorkerEvent::Heartbeat).unwrap();
        let emitter = BatchEmitter { sender };
        let cancellation = CancellationToken::default();
        cancellation.cancel();

        let error = emitter
            .send_cancellable(
                RecordBatch::new_empty(Arc::new(Schema::empty())),
                &cancellation,
                ErrorPhase::Read,
            )
            .unwrap_err();
        assert_eq!(error.category, plenora_io_model::ErrorCategory::Cancelled);
    }

    #[test]
    fn cancellable_send_exits_when_a_full_channel_is_cancelled() {
        let (sender, _receiver) = sync_channel(1);
        sender.send(BatchWorkerEvent::Heartbeat).unwrap();
        let cancellation = CancellationToken::default();
        let worker_cancellation = cancellation.clone();
        let worker = std::thread::spawn(move || {
            BatchEmitter { sender }.send_cancellable(
                RecordBatch::new_empty(Arc::new(Schema::empty())),
                &worker_cancellation,
                ErrorPhase::Read,
            )
        });

        cancellation.cancel();
        let error = worker.join().unwrap().unwrap_err();
        assert_eq!(error.category, plenora_io_model::ErrorCategory::Cancelled);
    }

    #[test]
    fn cancellable_send_exits_when_the_receiver_of_a_full_channel_drops() {
        let (sender, receiver) = sync_channel(1);
        sender.send(BatchWorkerEvent::Heartbeat).unwrap();
        let worker = std::thread::spawn(move || {
            BatchEmitter { sender }.send_cancellable(
                RecordBatch::new_empty(Arc::new(Schema::empty())),
                &CancellationToken::default(),
                ErrorPhase::Read,
            )
        });

        drop(receiver);
        assert!(!worker.join().unwrap().unwrap());
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
        assert!(matches!(
            reader.next_batch(),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Format
        ));
    }
}
