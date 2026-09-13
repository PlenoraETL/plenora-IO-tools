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
use plenora_io_model::PublicMessage;
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
    /// Segnale alzato entrando nel ramo di backpressure. Esiste solo nei test.
    ///
    /// Quel ramo si attraversa soltanto se il canale e' pieno **al momento del
    /// tentativo**, e senza un segnale legato al ramo reale una sonda puo' solo
    /// sperare di vincere una corsa: se il ricevente sparisce prima, il primo
    /// `try_send` vede `Disconnected`, la sonda passa lo stesso e il ramo non
    /// viene mai eseguito. Un timeout non deve poter trasformare una copertura
    /// mancata in un verde.
    ///
    /// E' per emettitore e non globale: due sonde che girassero in parallelo
    /// si scambierebbero i segnali, e l'attesa di una sarebbe soddisfatta dal
    /// ramo dell'altra.
    #[cfg(test)]
    entrato_in_backpressure: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl BatchEmitter {
    #[must_use]
    pub fn send(&self, batch: RecordBatch) -> bool {
        self.sender.send(BatchWorkerEvent::Batch(batch)).is_ok()
    }

    /// Consegna con backpressure senza rendere il producer cieco alla
    /// cancellazione mentre il canale bounded e' pieno.
    ///
    /// # Errors
    ///
    /// Restituisce l'errore di cancellazione se il token viene attivato
    /// durante l'attesa, oppure [`PlenoraIoError::format`] se il canale
    /// restituisce un evento diverso dal batch inviato.
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
                        #[cfg(test)]
                        if let Some(segnale) = &self.entrato_in_backpressure {
                            segnale.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        batch = returned;
                        std::thread::park_timeout(backoff);
                        backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
                    }
                    BatchWorkerEvent::Heartbeat
                    | BatchWorkerEvent::Finished
                    | BatchWorkerEvent::Failed(_) => {
                        return Err(PlenoraIoError::formato_redatto("batch-worker", &PublicMessage::Curated("il canale bounded ha restituito un evento diverso dal batch inviato")));
                    }
                },
            }
        }
    }

    /// Controlla senza bloccare se il consumer esiste ancora.
    #[must_use]
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
        PlenoraIoError::formato_redatto(
            self.driver,
            &PublicMessage::Curated("worker di lettura terminato senza stato terminale"),
        )
    }

    fn join_worker(&mut self) -> Result<()> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker.join().map_err(|_| {
            PlenoraIoError::formato_redatto(
                self.driver,
                &PublicMessage::Curated("worker di lettura terminato in modo anomalo"),
            )
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
                // Heartbeat: nessuno stato terminale, si riprova la ricezione.
                Ok(BatchWorkerEvent::Heartbeat) => {}
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
///
/// # Errors
///
/// Restituisce [`PlenoraIoError::format`] se il thread del worker non può
/// essere avviato.
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
            let result = catch_unwind(AssertUnwindSafe(|| {
                run(BatchEmitter {
                    sender,
                    #[cfg(test)]
                    entrato_in_backpressure: None,
                })
            }));
            let event = match result {
                Ok(Ok(())) => BatchWorkerEvent::Finished,
                Ok(Err(error)) => BatchWorkerEvent::Failed(error),
                Err(_) => BatchWorkerEvent::Failed(PlenoraIoError::formato_redatto(
                    driver,
                    &PublicMessage::Curated("worker di lettura terminato in modo anomalo"),
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
mod tests;
