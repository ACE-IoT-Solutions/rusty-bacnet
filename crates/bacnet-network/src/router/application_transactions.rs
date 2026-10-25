//! Which of the two stacks on the router's application entity owns each
//! confirmed request it sent, so that every answer reaches the stack that
//! asked (#2069).
//!
//! A client and a server on [split ports](super::RouterApplicationPorts)
//! both send confirmed requests from the one entity: the client its reads
//! and writes, the server its notifications, writes to Command targets and
//! binding probes. An answer names only the device it came from and the
//! invoke ID, and the two stacks pick their invoke IDs on their own, so the
//! table here keeps, for each request in progress, who sent it, to which
//! device, with which ID.
//!
//! When both stacks have a request open to one device under one invoke ID,
//! the later one goes out under another ID that device has free, and its
//! answers are given back under the ID its stack chose. Until then, an ID
//! goes out unchanged.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use bacnet_npdu::Address;
use bacnet_types::MacAddr;
use tokio::time::{Duration, Instant};

/// The most requests tracked at once; a new one past it pushes out the one
/// used least recently.
pub(super) const CAPACITY: usize = 1024;

/// How long a request is kept since it was last sent or answered: longer
/// than a stack waits for one answer before it retries or gives up.
pub(super) const LIFETIME: Duration = Duration::from_secs(60);

// docs/rust-api.md names both numbers: change them together.
const _: () = assert!(CAPACITY == 1024 && LIFETIME.as_secs() == 60);

/// The two stacks that share the entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    /// The stack that sent the request as a client, such as `BACnetClient`.
    Client,
    /// The stack that serves the entity, such as `BACnetServer`.
    Server,
}

/// One request in progress.
struct Open {
    side: Side,
    network: u16,
    mac: MacAddr,
    /// The invoke ID its stack chose.
    local: u8,
    /// The invoke ID it went out under.
    wire: u8,
    used_at: Instant,
}

impl Open {
    fn to(&self, network: u16, mac: &[u8]) -> bool {
        self.network == network && self.mac.as_slice() == mac
    }
}

/// The requests the two stacks have open, oldest use first.
#[derive(Default)]
pub(super) struct Transactions {
    open: Mutex<VecDeque<Open>>,
}

impl Transactions {
    fn open(&self) -> MutexGuard<'_, VecDeque<Open>> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        open.retain(|entry| now.duration_since(entry.used_at) < LIFETIME);
        open
    }

    /// The invoke ID a Confirmed-Request (or one segment of it) that `side`
    /// sends to `to` under its own `local` ID goes out under: the one the
    /// request already has, else `local` unless the other stack's request
    /// to that device holds it, else the next ID free there. `None` when
    /// every ID to that device is taken.
    pub(super) fn request(&self, side: Side, to: Address<'_>, local: u8) -> Option<u8> {
        let mut open = self.open();
        let now = Instant::now();
        if let Some(index) = open.iter().position(|entry| {
            entry.side == side && entry.to(to.network, to.mac) && entry.local == local
        }) {
            return Some(refresh(&mut open, index, now).wire);
        }
        let taken = |wire: u8| {
            open.iter()
                .any(|entry| entry.to(to.network, to.mac) && entry.wire == wire)
        };
        let wire = (0..=u8::MAX)
            .map(|step| local.wrapping_add(step))
            .find(|&wire| !taken(wire))?;
        if open.len() == CAPACITY {
            open.pop_front();
        }
        open.push_back(Open {
            side,
            network: to.network,
            mac: MacAddr::from_slice(to.mac),
            local,
            wire,
            used_at: now,
        });
        Some(wire)
    }

    /// The invoke ID a Segment-ACK or Abort that `side` sends as the
    /// requester, to `to` under its own `local` ID, goes out under: its
    /// request's, or `local` when it has none. An Abort (`ends`) closes the
    /// request.
    pub(super) fn requester(&self, side: Side, to: Address<'_>, local: u8, ends: bool) -> u8 {
        let mut open = self.open();
        let Some(index) = open.iter().position(|entry| {
            entry.side == side && entry.to(to.network, to.mac) && entry.local == local
        }) else {
            return local;
        };
        if ends {
            open.remove(index).map_or(local, |entry| entry.wire)
        } else {
            refresh(&mut open, index, Instant::now()).wire
        }
    }

    /// The stack whose request an answer from `network` and `mac` under
    /// `wire` answers, and the ID that stack chose; `None` when no open
    /// request matches. An answer that ends the request (`ends`) closes it.
    pub(super) fn answer(
        &self,
        network: u16,
        mac: &[u8],
        wire: u8,
        ends: bool,
    ) -> Option<(Side, u8)> {
        let mut open = self.open();
        let index = open
            .iter()
            .position(|entry| entry.to(network, mac) && entry.wire == wire)?;
        let entry = if ends {
            open.remove(index)?
        } else {
            let entry = refresh(&mut open, index, Instant::now());
            return Some((entry.side, entry.local));
        };
        Some((entry.side, entry.local))
    }

    /// How many requests are open now.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.open().len()
    }
}

/// Mark `open[index]` used at `now`, moving it to the back, and return it.
fn refresh(open: &mut VecDeque<Open>, index: usize, now: Instant) -> &Open {
    let mut entry = open.remove(index).expect("the index is in range");
    entry.used_at = now;
    open.push_back(entry);
    open.back().expect("just pushed")
}
