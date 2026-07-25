//! Contatori di osservabilità (decode/encode WKB), attivi solo con la feature
//! `metrics`. Nel build di produzione (senza feature) gli incrementi non esistono
//! e l'hot path resta minimale (V2).

use std::sync::atomic::{AtomicU64, Ordering};

pub static WKB_DECODE: AtomicU64 = AtomicU64::new(0);
pub static WKB_ENCODE: AtomicU64 = AtomicU64::new(0);

#[inline]
pub(crate) fn inc_decode() {
    WKB_DECODE.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub(crate) fn inc_encode() {
    WKB_ENCODE.fetch_add(1, Ordering::Relaxed);
}

/// (decode, encode) dall'ultimo reset.
pub fn snapshot() -> (u64, u64) {
    (
        WKB_DECODE.load(Ordering::Relaxed),
        WKB_ENCODE.load(Ordering::Relaxed),
    )
}

pub fn reset() {
    WKB_DECODE.store(0, Ordering::Relaxed);
    WKB_ENCODE.store(0, Ordering::Relaxed);
}
