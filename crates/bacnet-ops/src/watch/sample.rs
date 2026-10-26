//! Every value a point gets, as a stream per lease: the snapshot keeps
//! only the latest values and the change signal folds changes together,
//! so a front end that keeps a history or a log reads it here.

use std::time::SystemTime;

use tokio::sync::broadcast::{self, error::RecvError, error::TryRecvError};
use tokio::time::Instant;

use crate::format::RawRow;

/// How a sample's values came.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// The point's first read.
    First,
    /// A COV notification.
    Notification,
    /// A poll.
    Poll,
}

/// One set of values a point got.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// The properties as read or notified, or why the read gave none.
    pub values: Result<Vec<RawRow>, String>,
    /// How they came.
    pub via: Via,
    /// When they arrived.
    pub at: Instant,
    /// When they arrived, by the wall clock.
    pub time: SystemTime,
}

impl Sample {
    pub(super) fn new(values: Result<Vec<RawRow>, String>, via: Via) -> Self {
        Self {
            values,
            via,
            at: Instant::now(),
            time: SystemTime::now(),
        }
    }
}

/// A lease's samples since it asked ([`Lease::samples`](super::Lease::samples)).
/// It holds [`WatchOptions::samples`](super::WatchOptions::samples); a
/// reader that falls behind loses the oldest, counted in
/// [`dropped`](Self::dropped).
#[derive(Debug)]
pub struct Samples {
    receiver: broadcast::Receiver<Sample>,
    dropped: u64,
}

impl Samples {
    pub(super) fn new(receiver: broadcast::Receiver<Sample>) -> Self {
        Self {
            receiver,
            dropped: 0,
        }
    }

    /// The next sample waiting, if one is.
    pub fn try_next(&mut self) -> Option<Sample> {
        loop {
            match self.receiver.try_recv() {
                Ok(sample) => return Some(sample),
                Err(TryRecvError::Lagged(missed)) => self.dropped += missed,
                Err(TryRecvError::Empty | TryRecvError::Closed) => return None,
            }
        }
    }

    /// Wait for the next sample; `None` once the point is no longer
    /// watched and every sample was read. Cancel safe.
    pub async fn next(&mut self) -> Option<Sample> {
        loop {
            match self.receiver.recv().await {
                Ok(sample) => return Some(sample),
                Err(RecvError::Lagged(missed)) => self.dropped += missed,
                Err(RecvError::Closed) => return None,
            }
        }
    }

    /// Samples lost because this reader fell behind.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}
