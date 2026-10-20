//! The receiving end of a Rust broadcast stream behind a Python async
//! iterator: SC connection events (#1829), I-Haves and network reports
//! (#1850), and COV notifications.
//!
//! The Rust senders never wait for a receiver: one that falls behind the
//! channel's capacity loses the oldest items. The Python iterators keep
//! going past such a gap and count what was lost in `missed`, so a slow
//! Python consumer costs only its own items, never the sender's progress.
//! Holding one holds no sender, so once the owner stops and its senders
//! drop, the next item is the end of the iteration.
//!
//! An item whose `__anext__` future was cancelled while the item was being
//! delivered, as a timed-out `asyncio.wait_for` does, comes back through
//! [`PutBack`] into a one-item slot that the next `__anext__` returns first
//! (#1858). A second such item while the slot is full is counted in
//! `missed`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};

use pyo3::prelude::*;
use pyo3::IntoPyObjectExt;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::{Mutex, Notify};

use crate::py_async::PutBack;

#[cfg(test)]
#[path = "event_stream_tests.rs"]
mod tests;

/// A broadcast receiver shared by one Python iterator's pending
/// `__anext__` futures, with its redelivery slot and a running count of
/// the items lost.
pub(crate) struct LossyReceiver<T> {
    rx: Arc<Mutex<broadcast::Receiver<T>>>,
    slot: Arc<Redelivery>,
}

/// The one-item slot for an item a cancelled `__anext__` could not take.
#[derive(Default)]
pub(crate) struct Redelivery {
    /// A leaf lock: held only to swap the item, never across an await or a
    /// call into Python.
    item: StdMutex<Option<Py<PyAny>>>,
    /// Wakes a `next()` already waiting on the channel when the slot fills.
    filled: Notify,
    missed: AtomicU64,
}

impl Redelivery {
    fn item(&self) -> MutexGuard<'_, Option<Py<PyAny>>> {
        self.item.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn take(&self) -> Option<Py<PyAny>> {
        self.item().take()
    }
}

impl PutBack for Redelivery {
    fn put_back(&self, _py: Python<'_>, value: Py<PyAny>) {
        let refused = {
            let mut item = self.item();
            match *item {
                None => {
                    *item = Some(value);
                    None
                }
                Some(_) => Some(value),
            }
        };
        match refused {
            None => self.filled.notify_one(),
            // Dropped after the lock is released, with the GIL held.
            Some(value) => {
                drop(value);
                self.missed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// What `next()` yields: an item fresh from the channel, or one a cancelled
/// `__anext__` put back, already a Python object.
pub(crate) enum Next<T> {
    /// From the channel.
    Fresh(T),
    /// From the redelivery slot.
    Again(Py<PyAny>),
}

impl<T> Next<T> {
    /// Convert a fresh item; a put-back one is already converted.
    pub(crate) fn map<U>(self, f: impl FnOnce(T) -> U) -> Next<U> {
        match self {
            Self::Fresh(item) => Next::Fresh(f(item)),
            Self::Again(object) => Next::Again(object),
        }
    }
}

impl<'py, T: IntoPyObject<'py>> IntoPyObject<'py> for Next<T> {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match self {
            Self::Fresh(item) => item.into_bound_py_any(py),
            Self::Again(object) => Ok(object.into_bound(py)),
        }
    }
}

impl<T: Clone + Send + 'static> LossyReceiver<T> {
    pub(crate) fn new(rx: broadcast::Receiver<T>) -> Self {
        Self {
            rx: Arc::new(Mutex::new(rx)),
            slot: Arc::default(),
        }
    }

    /// Items lost so far: to lag, or to a full redelivery slot.
    pub(crate) fn missed(&self) -> u64 {
        self.slot.missed.load(Ordering::Relaxed)
    }

    /// The hook to hand `future_into_py_or_put_back`.
    pub(crate) fn put_back(&self) -> Arc<dyn PutBack> {
        self.slot.clone()
    }

    /// The item in the redelivery slot, else the next one from the channel;
    /// `None` once every sender is gone and the slot is empty. A lag adds
    /// the count lost to `missed` and goes on to the oldest item still held.
    ///
    /// Dropping this future before it yields loses no item: the lock,
    /// `recv` and `notified` are cancel-safe, and the slot is taken only on
    /// the poll that returns it. An item put back while this waits on the
    /// channel wakes it, so the put-back item comes first. The lock only
    /// orders this iterator's own concurrent `__anext__` calls; no Python
    /// thread waits on it.
    pub(crate) fn next(
        &self,
    ) -> impl std::future::Future<Output = Option<Next<T>>> + Send + 'static {
        let rx = Arc::clone(&self.rx);
        let slot = Arc::clone(&self.slot);
        async move {
            let mut rx = rx.lock().await;
            loop {
                if let Some(object) = slot.take() {
                    return Some(Next::Again(object));
                }
                tokio::select! {
                    biased;
                    () = slot.filled.notified() => {}
                    received = rx.recv() => match received {
                        Ok(item) => return Some(Next::Fresh(item)),
                        Err(RecvError::Lagged(lost)) => {
                            slot.missed.fetch_add(lost, Ordering::Relaxed);
                        }
                        Err(RecvError::Closed) => return slot.take().map(Next::Again),
                    },
                }
            }
        }
    }
}

/// `StopAsyncIteration`, for a stream whose owner has stopped.
pub(crate) fn ended(what: &str) -> pyo3::PyErr {
    pyo3::exceptions::PyStopAsyncIteration::new_err(format!("{what} ended: the client stopped"))
}
