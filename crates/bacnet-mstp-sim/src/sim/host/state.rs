//! Buffered state is owned by one powered incarnation, never queued globally.
use std::collections::VecDeque;

use bacnet_mstp_core::{Duration, Instant};

use super::echo::EchoFilter;
use crate::line::Character;
use crate::report::HostStats;

/// A serial write can outlive physical driver release; its completion stamp
/// remains the later of drain return and wire end, even when echo waits.
pub(super) struct Write {
    pub ended: Instant,
    pub frame: Vec<u8>,
    pub hunting: bool,
}

pub(crate) struct HostState {
    pub(super) input: VecDeque<Character>,
    pub(super) ready: Option<Instant>,
    pub(super) write: Option<Write>,
    pub(super) filter: EchoFilter,
    pub(crate) stats: HostStats,
    pub(crate) activity: Option<Instant>,
    /// A deferred completion can expose an already overdue core timer.
    pub(crate) completion_reported: Option<Instant>,
    /// Cumulative receive oracle, deferred until input has reached the core.
    pub(super) expected: VecDeque<(Instant, u32)>,
}

impl HostState {
    pub(crate) fn unread_until(&self) -> Option<Instant> {
        self.ready.filter(|_| !self.input.is_empty())
    }

    pub(crate) fn new(frame_abort: Duration) -> Self {
        Self {
            input: VecDeque::new(),
            ready: None,
            write: None,
            filter: EchoFilter::new(frame_abort),
            stats: HostStats::default(),
            activity: None,
            expected: VecDeque::new(),
            completion_reported: None,
        }
    }

    pub(crate) fn next(&self) -> Option<Instant> {
        if let Some(write) = &self.write {
            if !write.hunting {
                return Some(write.ended);
            }
        }
        self.ready
            .into_iter()
            .chain(self.filter.deadline())
            .chain(self.expected.front().map(|(at, _)| *at))
            .min()
    }
}
