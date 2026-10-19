//! The receiving end of a Rust broadcast stream behind a Python async
//! iterator: SC connection events (#1829), I-Haves and network reports
//! (#1850).
//!
//! The Rust senders never wait for a receiver: one that falls behind the
//! channel's capacity loses the oldest items. The Python iterators keep
//! going past such a gap and count what was lost in `missed`, so a slow
//! Python consumer costs only its own items, never the sender's progress.
//! Holding one holds no sender, so once the owner stops and its senders
//! drop, the next item is the end of the iteration.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::Mutex;

#[cfg(test)]
#[path = "event_stream_tests.rs"]
mod tests;

/// A broadcast receiver shared by one Python iterator's pending
/// `__anext__` futures, with a running count of the items lost to lag.
pub(crate) struct LossyReceiver<T> {
    rx: Arc<Mutex<broadcast::Receiver<T>>>,
    missed: Arc<AtomicU64>,
}

impl<T: Clone + Send + 'static> LossyReceiver<T> {
    pub(crate) fn new(rx: broadcast::Receiver<T>) -> Self {
        Self {
            rx: Arc::new(Mutex::new(rx)),
            missed: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Items lost so far because this receiver fell behind.
    pub(crate) fn missed(&self) -> u64 {
        self.missed.load(Ordering::Relaxed)
    }

    /// The next item, or `None` once every sender is gone. A lag adds the
    /// count lost to `missed` and goes on to the oldest item still held.
    ///
    /// Cancel-safe: dropping the future (a cancelled `__anext__`) loses no
    /// item, since both the lock and `recv` are. The lock only orders this
    /// iterator's own concurrent `__anext__` calls; no Python thread waits
    /// on it.
    pub(crate) fn next(&self) -> impl std::future::Future<Output = Option<T>> + Send + 'static {
        let rx = Arc::clone(&self.rx);
        let missed = Arc::clone(&self.missed);
        async move {
            let mut rx = rx.lock().await;
            loop {
                match rx.recv().await {
                    Ok(item) => return Some(item),
                    Err(RecvError::Lagged(lost)) => {
                        missed.fetch_add(lost, Ordering::Relaxed);
                    }
                    Err(RecvError::Closed) => return None,
                }
            }
        }
    }
}

/// `StopAsyncIteration`, for a stream whose owner has stopped.
pub(crate) fn ended(what: &str) -> pyo3::PyErr {
    pyo3::exceptions::PyStopAsyncIteration::new_err(format!("{what} ended: the client stopped"))
}
