//! The stack's `tracing` events, forwarded to Python's `logging` (#1724).
//!
//! Importing the module installs [`Bridge`] as the global `tracing`
//! subscriber. An event at target `bacnet_transport::bip` goes to the logger
//! `rusty_bacnet.bacnet_transport.bip`; ERROR, WARN, INFO and DEBUG keep
//! their names, and TRACE goes out at level 5, named `TRACE`.
//!
//! **Cost when off.** Whether a logger takes a level is read from a snapshot
//! of the `logging` configuration ([`levels`]), never from Python, so a
//! binding thread decides without the GIL. Each callsite asks once, and
//! `tracing` caches the answer in the callsite as an atomic `Interest`; a
//! disabled event then costs one atomic load, or less when its level is above
//! every logger's (the subscriber's level hint). `refresh_logging()` reads the
//! configuration again and rebuilds those caches.
//!
//! **Thread safety.** An enabled event is formatted on the thread that emits
//! it and queued; it never takes the GIL there. That thread may hold any of
//! the stack's locks, and a Python thread may be waiting on one of them with
//! the GIL held, so taking the GIL there could deadlock, and it would stall a
//! Tokio worker. One forwarder thread drains the queue and calls `logging`
//! through [`py_async::attach`], holding no lock of ours, so a handler that
//! calls back into `rusty_bacnet` (and logs again) only queues more records;
//! once the interpreter is exiting, records are dropped. A full queue drops
//! records too, and the next delivery reports how many with a warning.
//!
//! The stack's throttled warnings (#1624, #1712) throttle before they log, so
//! they reach Python as rarely as they reach any subscriber.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::time::{SystemTime, UNIX_EPOCH};

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use pyo3::{intern, IntoPyObjectExt};
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::span;
use tracing::subscriber::Interest;
use tracing::{Dispatch, Event, Metadata, Subscriber};

use crate::py_async;

mod levels;
#[cfg(test)]
mod tests;

use levels::{logger_name, python_level, Levels, PREFIX};

/// Records waiting for the forwarder; past this, new ones are dropped.
const QUEUE: usize = 4096;

/// Records delivered per attach, so a flood doesn't hold the GIL for long.
const BATCH: usize = 64;

/// Records dropped because the queue was full, not yet reported.
static DROPPED: AtomicU64 = AtomicU64::new(0);

/// Records queued, for the tests.
#[cfg(test)]
static QUEUED: AtomicU64 = AtomicU64::new(0);

/// Register `refresh_logging` and install the bridge.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(refresh_logging, m)?)?;
    install(m.py())
}

/// Read the `logging` configuration again, so that level changes made since
/// import (or the last refresh) decide which events are forwarded.
#[pyfunction]
pub(crate) fn refresh_logging(py: Python<'_>) -> PyResult<()> {
    levels::store(Levels::read(py)?);
    // Rebuilding locks tracing's callsite registry, which a thread holding
    // the GIL may be waiting on; so release it.
    py.detach(tracing::callsite::rebuild_interest_cache);
    Ok(())
}

/// Install the bridge as the global subscriber, once.
///
/// This module's copy of `tracing` is its own: no other extension module,
/// and no Python code, can set its global subscriber, so this only finds one
/// set when a Rust embedder (or a test) set it first. Then that subscriber
/// stays, and nothing is forwarded.
pub(crate) fn install(py: Python<'_>) -> PyResult<()> {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    levels::name_trace_level(py)?;
    levels::store(Levels::read(py)?);
    let (queue, records) = sync_channel(QUEUE);
    std::thread::Builder::new()
        .name("rusty-bacnet-logging".into())
        .spawn(move || forward_all(&records))
        .map_err(|e| PyRuntimeError::new_err(format!("start the logging thread: {e}")))?;
    // If a subscriber is set already, the dispatch and its sender are
    // dropped, and the forwarder thread ends.
    py.detach(|| tracing::dispatcher::set_global_default(Dispatch::new(Bridge { queue })))
        .ok();
    Ok(())
}

/// The subscriber: decides from [`levels`], queues what's enabled.
struct Bridge {
    queue: SyncSender<Record>,
}

/// Whether the logger for `metadata` takes it, by the current snapshot.
fn wanted(metadata: &Metadata<'_>) -> bool {
    metadata.is_event()
        && levels::current().enabled(
            &logger_name(metadata.target()),
            python_level(*metadata.level()),
        )
}

impl Subscriber for Bridge {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if wanted(metadata) {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        wanted(metadata)
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(levels::current().max_level())
    }

    // Spans are never enabled, so these are never called with one.
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let record = Record {
            metadata: event.metadata(),
            message: fields.message,
            fields: fields.fields,
            created: SystemTime::now(),
        };
        match self.queue.try_send(record) {
            Ok(()) => {
                #[cfg(test)]
                QUEUED.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Full(_)) => {
                DROPPED.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    fn enter(&self, _: &span::Id) {}

    fn exit(&self, _: &span::Id) {}
}

/// One event, ready for `logging`.
struct Record {
    metadata: &'static Metadata<'static>,
    message: Option<String>,
    fields: Vec<(&'static str, Value)>,
    created: SystemTime,
}

/// A field's value, kept as a Python type where one fits.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Str(String),
    I64(i64),
    U64(u64),
    F64(f64),
    Bool(bool),
    /// Anything else, by its `Debug` (or, for `%field`, `Display`) form.
    Text(String),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Str(s) | Self::Text(s) => f.write_str(s),
            Self::I64(n) => write!(f, "{n}"),
            Self::U64(n) => write!(f, "{n}"),
            Self::F64(n) => write!(f, "{n}"),
            Self::Bool(b) => write!(f, "{b}"),
        }
    }
}

impl Value {
    fn to_python<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match self {
            Self::Str(s) | Self::Text(s) => s.into_bound_py_any(py),
            Self::I64(n) => n.into_bound_py_any(py),
            Self::U64(n) => n.into_bound_py_any(py),
            Self::F64(n) => n.into_bound_py_any(py),
            Self::Bool(b) => b.into_bound_py_any(py),
        }
    }
}

/// Collects an event's message and fields.
#[derive(Default)]
struct Fields {
    message: Option<String>,
    fields: Vec<(&'static str, Value)>,
}

impl Fields {
    fn put(&mut self, field: &Field, value: Value) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push((field.name(), value));
        }
    }
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.put(field, Value::Text(format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, Value::Str(value.to_owned()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, Value::I64(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, Value::U64(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(field, Value::F64(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, Value::Bool(value));
    }
}

/// The record's message: the event's message, then each field as
/// `key=value`.
fn message_text(message: Option<&str>, fields: &[(&'static str, Value)]) -> String {
    let mut text = message.unwrap_or_default().to_owned();
    for (name, value) in fields {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(name);
        text.push('=');
        text.push_str(&value.to_string());
    }
    text
}

/// The forwarder thread: hand queued records to `logging` until the sending
/// side is gone.
fn forward_all(records: &Receiver<Record>) {
    while let Ok(first) = records.recv() {
        let batch: Vec<Record> = std::iter::once(first)
            .chain(records.try_iter().take(BATCH - 1))
            .collect();
        let dropped = DROPPED.swap(0, Ordering::Relaxed);
        // Refused once the interpreter is exiting: the batch is dropped.
        let _ = py_async::attach(|py| {
            for record in batch {
                if let Err(err) = forward(py, record) {
                    err.write_unraisable(py, None);
                }
            }
            if dropped > 0 {
                if let Err(err) = report_dropped(py, dropped) {
                    err.write_unraisable(py, None);
                }
            }
            Ok(())
        });
    }
}

/// Hand one record to its logger, if the logger still takes its level.
fn forward(py: Python<'_>, record: Record) -> PyResult<()> {
    let metadata = record.metadata;
    let name = logger_name(metadata.target());
    let level = python_level(*metadata.level());
    let logger = py
        .import(intern!(py, "logging"))?
        .call_method1(intern!(py, "getLogger"), (&name,))?;
    // A level raised since the last refresh still filters here.
    if !logger
        .call_method1(intern!(py, "isEnabledFor"), (level,))?
        .is_truthy()?
    {
        return Ok(());
    }
    let fields = PyDict::new(py);
    for (key, value) in &record.fields {
        fields.set_item(key, value.to_python(py)?)?;
    }
    let extra = PyDict::new(py);
    extra.set_item(intern!(py, "bacnet_fields"), fields)?;
    extra.set_item(intern!(py, "bacnet_target"), metadata.target())?;
    let text = message_text(record.message.as_deref(), &record.fields);
    let made = logger.call_method1(
        intern!(py, "makeRecord"),
        (
            &name,
            level,
            metadata.file().unwrap_or("(unknown file)"),
            metadata.line().unwrap_or(0),
            text,
            PyTuple::empty(py),
            py.None(),
            py.None(),
            extra,
        ),
    )?;
    // When the event happened, not when it was delivered.
    if let Ok(since) = record.created.duration_since(UNIX_EPOCH) {
        made.setattr(intern!(py, "created"), since.as_secs_f64())?;
        made.setattr(intern!(py, "msecs"), f64::from(since.subsec_millis()))?;
    }
    logger.call_method1(intern!(py, "handle"), (made,))?;
    Ok(())
}

/// Warn, on `rusty_bacnet`, that `dropped` records never reached `logging`.
fn report_dropped(py: Python<'_>, dropped: u64) -> PyResult<()> {
    py.import(intern!(py, "logging"))?
        .call_method1(intern!(py, "getLogger"), (PREFIX,))?
        .call_method1(
            intern!(py, "warning"),
            (
                "%d log records were dropped: the queue to logging was full",
                dropped,
            ),
        )?;
    Ok(())
}
