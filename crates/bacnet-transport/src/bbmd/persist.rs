//! The persisted-BDT file a BBMD starts from
//! ([`BipTransport::set_bdt_persist_path`](crate::bip::BipTransport::set_bdt_persist_path)),
//! and its writer: a BDT activated through the Network Port object is saved
//! there, so a restart keeps it (#1826).
//!
//! The file holds the BDT rows as a Write-BDT payload carries them, 10
//! octets each, as the loader reads it. A write goes to a temporary file
//! beside it, is synced, and then renamed over it, so a reader or a crash
//! sees the old table or the new one, never part of one. A failed write is
//! logged and counted, and leaves the activation in place: the BBMD runs on
//! the new table either way, and only a restart would lose it.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use bytes::BytesMut;
use tracing::{debug, warn};

use super::{encode_bdt_entries, BdtEntry};

/// One BBMD's persisted-BDT file.
#[derive(Debug)]
pub(crate) struct BdtFile {
    path: PathBuf,
    /// Held across a save, so saves land in the order their tables were
    /// taken: a slower, older save can't overwrite a newer one.
    order: Mutex<()>,
    /// Saves that failed.
    failures: AtomicU64,
}

impl BdtFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            order: Mutex::new(()),
            failures: AtomicU64::new(0),
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

    /// Take the order lock: the caller then takes the table under the BBMD
    /// lock, releases that, and saves it with [`save`](Self::save) before
    /// letting this go. The BBMD lock is never held across the file I/O.
    pub(crate) fn order(&self) -> std::sync::MutexGuard<'_, ()> {
        self.order.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Write `rows` to the file, replacing it whole; a failure is logged and
    /// counted.
    pub(crate) fn save(&self, rows: &[BdtEntry]) {
        match self.write(rows) {
            Ok(()) => {
                debug!(path = %self.path.display(), entries = rows.len(), "Saved the activated BDT")
            }
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

    fn write(&self, rows: &[BdtEntry]) -> std::io::Result<()> {
        let mut octets = BytesMut::new();
        encode_bdt_entries(rows, &mut octets);
        let name = self.path.file_name().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "the path names no file")
        })?;
        let mut temporary = name.to_owned();
        temporary.push(".tmp");
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
        written
    }
}
