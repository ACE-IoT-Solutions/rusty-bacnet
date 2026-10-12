//! Recognizing this station's own echo among the octets it reads, on a port
//! that declares [`Echo::Full`](super::Echo::Full).
//!
//! The filter compares each octet read after a frame with the next octet of
//! that frame. A matching octet is held back, not fed to the core, and the
//! held octets are dropped once the whole frame has matched: that was the
//! echo. The first octet that does not match ends the filter; the held
//! octets are released to the core, then that octet, in order. So an octet
//! is dropped only as part of a complete copy of the frame just sent, and a
//! peer's octets always reach the core, even when they arrive where the
//! echo should be, or start the way the frame did (every frame starts with
//! the same preamble).
//!
//! The transport starts the filter when a frame ends and keeps it on, past
//! the frame's completion if need be, until it matches or fails, or until
//! this station's next confirmed start. A filter started in probe mode
//! (after a grace with no echo at all) stops at its own time limit as well,
//! releasing what it holds.

use tokio::time::Instant;

/// What to do with one octet read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Filtered {
    /// Held back as possible echo: feed nothing now.
    Held,
    /// The last octet of the echo: it and the held octets are dropped.
    Echo,
    /// Not echo. Feed the first `held` octets of [`EchoFilter::held`], then
    /// this octet. `mismatch` says the filter had seen echo, so the echo
    /// differed from what was sent.
    Release { held: usize, mismatch: bool },
}

/// The echo filter for the frame this station sent last.
#[derive(Debug, Default)]
pub(super) struct EchoFilter {
    /// The frame's octets.
    expected: Vec<u8>,
    /// How many octets of `expected` have matched and are held back.
    matched: usize,
    /// Filtering, rather than passing everything through.
    on: bool,
    /// Probe mode: no echo was seen within the grace, so what matches may
    /// be a late echo or a peer's frame; filtering stops at this time.
    probe_until: Option<Instant>,
}

impl EchoFilter {
    /// Start matching the echo of `frame`, dropping anything still held.
    pub(super) fn start(&mut self, frame: &[u8]) {
        self.expected.clear();
        self.expected.extend_from_slice(frame);
        self.matched = 0;
        self.on = true;
        self.probe_until = None;
    }

    /// Stop filtering, dropping what is held: this station has started a
    /// new frame, so what matched the old one's echo was its echo.
    pub(super) fn clear(&mut self) {
        self.on = false;
        self.matched = 0;
        self.probe_until = None;
    }

    /// Whether any of the echo has been seen.
    pub(super) fn seen(&self) -> bool {
        self.matched > 0
    }

    /// Keep filtering past the grace with none of the echo seen, until
    /// `until`: a late echo is still dropped, and a peer's frame released.
    pub(super) fn probe(&mut self, until: Instant) {
        if self.on && self.matched == 0 {
            self.probe_until = Some(until);
        }
    }

    /// The held octets, in the order they were read.
    pub(super) fn held(&self, count: usize) -> &[u8] {
        &self.expected[..count]
    }

    /// End probe mode once its time is up: stop filtering and return how
    /// many held octets to release, first.
    pub(super) fn expire(&mut self, now: Instant) -> usize {
        if self.on && self.probe_until.is_some_and(|until| now >= until) {
            self.on = false;
            self.probe_until = None;
            return std::mem::take(&mut self.matched);
        }
        0
    }

    /// Filter one octet read.
    pub(super) fn push(&mut self, octet: u8) -> Filtered {
        if !self.on {
            return Filtered::Release {
                held: 0,
                mismatch: false,
            };
        }
        if self.expected.get(self.matched) == Some(&octet) {
            self.matched += 1;
            if self.matched == self.expected.len() {
                self.clear();
                return Filtered::Echo;
            }
            return Filtered::Held;
        }
        let held = std::mem::take(&mut self.matched);
        let mismatch = self.probe_until.is_none();
        self.on = false;
        self.probe_until = None;
        Filtered::Release { held, mismatch }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: [u8; 8] = [0x55, 0xFF, 0x00, 0x04, 0x03, 0x00, 0x00, 0x8F];

    fn run(filter: &mut EchoFilter, octets: &[u8]) -> Vec<u8> {
        let mut fed = Vec::new();
        for &octet in octets {
            if let Filtered::Release { held, .. } = filter.push(octet) {
                fed.extend_from_slice(filter.held(held));
                fed.push(octet);
            }
        }
        fed
    }

    #[test]
    fn a_whole_echo_is_dropped_and_what_follows_is_fed() {
        let mut filter = EchoFilter::default();
        filter.start(&FRAME);
        let mut octets = FRAME.to_vec();
        octets.extend_from_slice(&[0x55, 0xFF, 0x00]);
        assert_eq!(run(&mut filter, &octets), [0x55, 0xFF, 0x00]);
    }

    #[test]
    fn a_frame_that_only_starts_like_the_echo_is_fed_whole() {
        let mut filter = EchoFilter::default();
        filter.start(&FRAME);
        let peer = [0x55, 0xFF, 0x00, 0x03, 0x04, 0x00, 0x00, 0x31];
        assert_eq!(run(&mut filter, &peer), peer);
        assert_eq!(
            {
                filter.start(&FRAME);
                filter.push(0x55);
                filter.push(0x00)
            },
            Filtered::Release {
                held: 1,
                mismatch: true
            }
        );
    }

    #[test]
    fn a_short_echo_is_fed_and_a_corrupted_one_released_from_the_start() {
        let mut filter = EchoFilter::default();
        filter.start(&FRAME);
        // The echo's first three octets are missing.
        assert_eq!(run(&mut filter, &FRAME[3..]), &FRAME[3..]);
        filter.start(&FRAME);
        let mut corrupted = FRAME;
        corrupted[4] ^= 0x40;
        assert_eq!(run(&mut filter, &corrupted), corrupted);
    }

    #[test]
    fn probe_mode_releases_without_a_mismatch_and_expires() {
        let start = Instant::now();
        let mut filter = EchoFilter::default();
        filter.start(&FRAME);
        filter.probe(start);
        assert_eq!(filter.push(0x55), Filtered::Held);
        assert_eq!(
            filter.push(0x01),
            Filtered::Release {
                held: 1,
                mismatch: false
            }
        );
        filter.start(&FRAME);
        filter.probe(start);
        filter.push(0x55);
        assert_eq!(filter.expire(start), 1);
        assert_eq!(
            filter.push(0xFF),
            Filtered::Release {
                held: 0,
                mismatch: false
            }
        );
    }

    #[test]
    fn a_new_frame_clears_what_is_held() {
        let mut filter = EchoFilter::default();
        filter.start(&FRAME);
        filter.push(0x55);
        assert!(filter.seen());
        filter.clear();
        assert!(!filter.seen());
        assert_eq!(
            filter.push(0xFF),
            Filtered::Release {
                held: 0,
                mismatch: false
            }
        );
    }
}
