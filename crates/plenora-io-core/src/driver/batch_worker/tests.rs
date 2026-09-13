//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

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
        Err(PlenoraIoError::limite_redatto(&PublicMessage::Curated(
            "limite del parser",
        )))
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
    let emitter = BatchEmitter {
        sender,
        entrato_in_backpressure: None,
    };
    drop(receiver);

    assert!(!emitter.is_receiver_alive());
}

#[test]
fn cancellable_send_observes_pre_cancelled_token_on_a_full_channel() {
    let (sender, _receiver) = sync_channel(1);
    sender.send(BatchWorkerEvent::Heartbeat).unwrap();
    let emitter = BatchEmitter {
        sender,
        entrato_in_backpressure: None,
    };
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
        BatchEmitter {
            sender,
            entrato_in_backpressure: None,
        }
        .send_cancellable(
            RecordBatch::new_empty(Arc::new(Schema::empty())),
            &worker_cancellation,
            ErrorPhase::Read,
        )
    });

    cancellation.cancel();
    let error = worker.join().unwrap().unwrap_err();
    assert_eq!(error.category, plenora_io_model::ErrorCategory::Cancelled);
}

/// Il ramo di backpressure, **deterministicamente**.
///
/// Le due sonde qui accanto lo attraversano solo se il produttore arriva a
/// `try_send` prima che l'altro thread cancelli o rilasci il ricevente. Se
/// perde quella corsa il ramo non si esegue, la sonda passa lo stesso, e la
/// copertura di quelle righe cambia fra due misure sullo stesso albero.
///
/// Qui non c'e' alcun timeout, e non c'e' corsa da vincere. Il canale ha
/// **capacita' zero**: `try_send` riesce solo se un ricevente e' gia'
/// parcheggiato in `recv`, e questa sonda non chiama mai `recv`, quindi il
/// tentativo trova il canale pieno finche' il ricevente esiste. Il
/// ricevente viene rilasciato **soltanto dopo** che il produttore ha
/// segnalato di essere entrato nel ramo: l'ordine non dipende dallo
/// scheduling, dipende dal ramo stesso.
///
/// Cio' che si prova non e' la copertura di sei righe: e' che un canale
/// pieno **non perde il batch e non diventa un errore del dataset** — il
/// produttore ritenta finche' il ricevente esiste, e riferisce `false`
/// quando sparisce.
#[test]
fn cancellable_send_ritenta_su_un_canale_a_rendezvous_finche_il_ricevente_esiste() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let (sender, receiver) = sync_channel(0);
    let entrato = Arc::new(AtomicBool::new(false));
    let emitter = BatchEmitter {
        sender,
        entrato_in_backpressure: Some(entrato.clone()),
    };

    let produttore = std::thread::spawn(move || {
        emitter.send_cancellable(
            RecordBatch::new_empty(Arc::new(Schema::empty())),
            &CancellationToken::default(),
            ErrorPhase::Read,
        )
    });

    // Attesa senza scadenza: il produttore **deve** entrare nel ramo,
    // perche' finche' il ricevente vive il canale a rendezvous e' pieno.
    // Una scadenza qui potrebbe rilasciare il ricevente troppo presto e
    // rendere verde una sonda che non ha attraversato niente.
    while !entrato.load(Ordering::SeqCst) {
        std::hint::spin_loop();
    }
    drop(receiver);

    assert!(
        !produttore.join().unwrap().unwrap(),
        "il ricevente sparito non e' un errore del dataset: e' un `false`"
    );
}

#[test]
fn cancellable_send_exits_when_the_receiver_of_a_full_channel_drops() {
    let (sender, receiver) = sync_channel(1);
    sender.send(BatchWorkerEvent::Heartbeat).unwrap();
    let worker = std::thread::spawn(move || {
        BatchEmitter {
            sender,
            entrato_in_backpressure: None,
        }
        .send_cancellable(
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
