//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore esattamente
//! come quando stavano dentro di lui, e non allargano di una riga la
//! superficie pubblica del crate.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::task::Wake;

#[derive(Debug)]
struct WakeFlag(AtomicBool);

impl Wake for WakeFlag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, AtomicOrdering::Release);
    }
}

#[test]
fn cancellation_is_idempotent_and_propagates_to_children() {
    let parent = CancellationToken::new();
    let child = parent.child_token();
    parent.cancel();
    parent.cancel();
    assert_eq!(parent.reason(), Some(CancellationReason::Requested));
    assert_eq!(child.reason(), Some(CancellationReason::Parent));
}

#[test]
fn cancelled_future_is_woken_without_polling() {
    let token = CancellationToken::new();
    let flag = Arc::new(WakeFlag(AtomicBool::new(false)));
    let waker = Waker::from(Arc::clone(&flag));
    let mut context = Context::from_waker(&waker);
    let mut future = token.cancelled();
    assert!(Pin::new(&mut future).poll(&mut context).is_pending());
    token.cancel();
    assert!(flag.0.load(AtomicOrdering::Acquire));
    assert_eq!(
        Pin::new(&mut future).poll(&mut context),
        Poll::Ready(CancellationReason::Requested)
    );
}

/// La scadenza dichiarata sul genitore arriva ai figli come scadenza, sia per
/// i figli che esistevano gia' sia per quelli creati dopo: era `Parent` per i
/// primi e `Deadline` per i secondi, e la categoria dipendeva dall'ordine.
#[test]
fn deadline_cancellation_propagates_as_deadline() {
    let parent = CancellationToken::new();
    let before = parent.child_token();
    let grandchild = before.child_token();
    parent.cancel_due_to_deadline();
    let after = parent.child_token();
    assert_eq!(parent.reason(), Some(CancellationReason::Deadline));
    assert_eq!(before.reason(), Some(CancellationReason::Deadline));
    assert_eq!(grandchild.reason(), Some(CancellationReason::Deadline));
    assert_eq!(after.reason(), Some(CancellationReason::Deadline));
}

#[test]
fn deadline_is_declarative() {
    let token = CancellationToken::with_deadline(Instant::now());
    assert_eq!(token.reason(), Some(CancellationReason::Deadline));
}
