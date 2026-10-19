//! The persisted-BDT file a BBMD starts from
//! ([`BipTransport::set_bdt_persist_path`](crate::bip::BipTransport::set_bdt_persist_path)),
//! and its writer: a BDT activated through the Network Port object is saved
//! there, so a restart keeps it (#1826).
//!
//! The file holds the BDT rows as a Write-BDT payload carries them, 10
//! octets each, as the loader reads it. A write goes to a temporary file
//! beside it, is synced, and then renamed over it, so a reader or a crash
//! sees the old table or the new one, never part of one; on Unix the
//! directory is synced too, so the rename itself survives a crash.
//!
//! An activation runs under the server's database lock and the BBMD lock,
//! so the file I/O happens after both are released, on a blocking thread:
//! the activation takes a generation for its table under the BBMD lock
//! ([`BdtFile::next_generation`]), and a save older than one already
//! written is skipped, so a slow save of an older table never overwrites a
//! newer one. A failed write is logged and counted, and leaves the
//! activation in place: the BBMD runs on the new table either way, and only
//! a restart would lose it.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bytes::BytesMut;
use tracing::{debug, warn};

use super::{encode_bdt_entries, BdtEntry};

/// One BBMD's persisted-BDT file.
#[derive(Debug)]
pub(crate) struct BdtFile {
    path: PathBuf,
    /// The newest generation handed out ([`Self::next_generation`]).
    generation: AtomicU64,
    /// Held across a save: the newest generation a save was made for.
    written: Mutex<u64>,
    /// Saves that failed.
    failures: AtomicU64,
    /// Saves that finished, written, failed or skipped.
    finished: AtomicU64,
}

impl BdtFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            generation: AtomicU64::new(0),
            written: Mutex::new(0),
            failures: AtomicU64::new(0),
            finished: AtomicU64::new(0),
        }
    }

    /// Where the file is.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// How many saves have failed.
    pub(crate) fn failures(&self) -> u64 {
        self.failures.load(Ordering::Relaxed)
    }

    /// How many saves have finished, whatever came of them.
    #[cfg(test)]
    pub(crate) fn finished(&self) -> u64 {
        self.finished.load(Ordering::Acquire)
    }

    /// The generation of a table about to be saved. Take it under the BBMD
    /// lock, with the table, so generations follow the order the tables
    /// went into effect.
    pub(crate) fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Save `rows`, of `generation`, off the caller's thread: on a blocking
    /// thread of the current Tokio runtime, or here with none running.
    pub(crate) fn save_later(self: &Arc<Self>, generation: u64, rows: Vec<BdtEntry>) {
        let file = Arc::clone(self);
        let save = move || file.save(generation, &rows);
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => drop(runtime.spawn_blocking(save)),
            Err(_) => save(),
        }
    }

    /// Write `rows`, of `generation`, to the file, replacing it whole,
    /// unless a newer generation was already saved; a failure is logged and
    /// counted.
    pub(crate) fn save(&self, generation: u64, rows: &[BdtEntry]) {
        let mut written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
        if generation <= *written {
            debug!(generation, newest = *written, "Skipped saving an older BDT");
        } else {
            *written = generation;
            match self.write(rows) {
                Ok(()) => debug!(
                    path = %self.path.display(),
                    entries = rows.len(),
                    "Saved the activated BDT"
                ),
                Err(error) => {
                    self.failures.fetch_add(1, Ordering::Relaxed);
                    warn!(
                        path = %self.path.display(),
                        %error,
                        "Failed to save the activated BDT; the BBMD uses it, but a restart will not"
                    );
                }
            }
        }
        drop(written);
        self.finished.fetch_add(1, Ordering::Release);
    }

    fn write(&self, rows: &[BdtEntry]) -> std::io::Result<()> {
        static TEMPORARY: AtomicU64 = AtomicU64::new(0);
        let mut octets = BytesMut::new();
        encode_bdt_entries(rows, &mut octets);
        let name = self.path.file_name().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "the path names no file")
        })?;
        // Unique to this process and save, so two writers of one path never
        // share a temporary file.
        let mut temporary = name.to_owned();
        temporary.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            TEMPORARY.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary = self.path.with_file_name(temporary);
        let written = (|| {
            let mut file = std::fs::File::create(&temporary)?;
            file.write_all(&octets)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &self.path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written?;
        sync_directory(&self.path);
        Ok(())
    }
}

/// Sync the directory holding `path`, so a rename into it survives a crash;
/// best effort.
#[cfg(unix)]
fn sync_directory(path: &Path) {
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    if let Err(error) = std::fs::File::open(directory).and_then(|dir| dir.sync_all()) {
        debug!(%error, "Could not sync the persisted BDT's directory");
    }
}

/// Other platforms offer no portable directory sync; skipped there.
#[cfg(not(unix))]
fn sync_directory(_path: &Path) {}
