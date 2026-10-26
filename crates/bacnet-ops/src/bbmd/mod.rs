//! B/IP broadcast management (Annex J) as plain data: BBMD table rows, what
//! reading a table gave, the breadth-first survey that follows each BDT to
//! the BBMDs it lists, and the consistency matrix over the tables read
//! ([`matrix`]). The TUI's Network screen and the MCP server's network map
//! share it; [`Engine`](crate::engine::Engine) reads the tables.
//!
//! Only the read side of BVLL is used: Read-Broadcast-Distribution-Table
//! and Read-Foreign-Device-Table (Annex J.2.3 and J.2.7). Nothing here
//! writes a table or registers.

pub mod matrix;

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_transport::bbmd::{BdtEntry, FdtEntryWire};
use bacnet_transport::bip::BvlcExchangeError;
use bacnet_transport::bvll::BvlcResultCode;
use futures_util::stream::{FuturesUnordered, StreamExt};

pub use self::matrix::{Cell, Finding, Matrix, Probe};

/// The most BBMDs a survey reads. A larger set comes back with the rest
/// unread ([`Outcome::Skipped`]).
pub const MAX_SURVEY: usize = 16;

/// The most BBMDs a survey reads at once. Each read takes one of the B/IP
/// transport's eight exchange slots (one per peer and request), so this
/// leaves room for the foreign-device registration and other reads, and
/// silent peers wait out their timeouts together.
pub const SURVEY_AT_ONCE: usize = 4;

/// The most entries a front end keeps of one table: the workspace's BDT and
/// FDT caps.
pub const MAX_ENTRIES: usize = 128;

/// An FDT entry with less than this many seconds left is near expiry. The
/// BBMD keeps an entry 30 s past its time-to-live (Annex J.5.2.3), so under
/// this the device is already past its own time-to-live without having
/// registered again.
pub const FDT_EXPIRING_S: u16 = 30;

/// One BDT entry: a BBMD and the mask its peers forward to it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BdtRow {
    /// The BBMD's B/IP address.
    pub address: SocketAddrV4,
    /// Its broadcast distribution mask.
    pub mask: Ipv4Addr,
}

impl BdtRow {
    /// True for an all-ones mask: peers send forwarded broadcasts to the
    /// BBMD itself, which broadcasts them on its own subnet (Annex J.4.3.1,
    /// the two-hop method).
    pub fn is_unicast(&self) -> bool {
        self.mask == Ipv4Addr::BROADCAST
    }

    /// Where a peer sends a forwarded broadcast for this entry: the address
    /// with every bit the mask clears set (Annex J.4.5). With an all-ones
    /// mask that is the BBMD itself.
    pub fn forward_to(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.address.ip().to_bits() | !self.mask.to_bits())
    }

    /// One line on what the mask means for forwarding.
    pub fn explain(&self) -> String {
        if self.is_unicast() {
            "peers unicast to it, and it rebroadcasts on its subnet (two-hop)".into()
        } else {
            format!(
                "peers send a directed broadcast to {} (one-hop; routers must pass it)",
                self.forward_to()
            )
        }
    }
}

impl From<&BdtEntry> for BdtRow {
    fn from(entry: &BdtEntry) -> Self {
        Self {
            address: SocketAddrV4::new(Ipv4Addr::from(entry.ip), entry.port),
            mask: Ipv4Addr::from(entry.broadcast_mask),
        }
    }
}

/// One FDT entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdtRow {
    /// The address the BBMD recorded for the foreign device, which is the
    /// source address its registration arrived from.
    pub address: SocketAddrV4,
    /// The time-to-live the device asked for, in seconds.
    pub ttl: u16,
    /// Seconds left before the BBMD drops the entry, as of the read. The
    /// BBMD adds a grace period to the time-to-live (Annex J.5.2.3).
    pub remaining: u16,
}

impl FdtRow {
    /// Whether the entry is near expiry: under [`FDT_EXPIRING_S`] left.
    pub fn expiring(&self) -> bool {
        self.remaining < FDT_EXPIRING_S
    }
}

impl From<&FdtEntryWire> for FdtRow {
    fn from(entry: &FdtEntryWire) -> Self {
        Self {
            address: SocketAddrV4::new(Ipv4Addr::from(entry.ip), entry.port),
            ttl: entry.ttl,
            remaining: entry.seconds_remaining,
        }
    }
}

/// The 6-octet B/IP MAC of `address`.
pub fn mac(address: SocketAddrV4) -> [u8; 6] {
    let [a, b, c, d] = address.ip().octets();
    let [high, low] = address.port().to_be_bytes();
    [a, b, c, d, high, low]
}

/// The address in a 6-octet B/IP MAC.
pub fn address_of(mac: &[u8]) -> Option<SocketAddrV4> {
    let [a, b, c, d, high, low] = <[u8; 6]>::try_from(mac).ok()?;
    Some(SocketAddrV4::new(
        Ipv4Addr::new(a, b, c, d),
        u16::from_be_bytes([high, low]),
    ))
}

/// What reading one of a BBMD's tables gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome<R> {
    /// The table.
    Table(R),
    /// It answered with this NAK result: it doesn't carry out the read, as
    /// a node that is no BBMD answers.
    Nak(BvlcResultCode),
    /// It did not answer, or the request could not be sent.
    Unreachable(String),
    /// Not read: the survey reached its limit, or the caller's budget ran
    /// out first.
    Skipped,
}

impl<R> Outcome<R> {
    /// The table, when one was read.
    pub fn table(&self) -> Option<&R> {
        match self {
            Self::Table(rows) => Some(rows),
            _ => None,
        }
    }

    /// Sort a failed table read into a refusal or no answer (#1822): a NAK
    /// is a refusal, while anything else (no answer, a reply the read does
    /// not take, a send failure, a malformed reply) means the BBMD could not
    /// be read.
    pub fn from_error(error: &BvlcExchangeError) -> Self {
        match error {
            BvlcExchangeError::Nak(code) => Self::Nak(*code),
            other => Self::Unreachable(unreachable_reason(other)),
        }
    }
}

/// [`Outcome::from_error`] for a BDT read.
pub fn probe_error(error: &BvlcExchangeError) -> Probe {
    Probe::from_error(error)
}

/// [`unreachable_reason`] for a BBMD that didn't answer in time.
pub const NO_ANSWER: &str = "no answer";

/// Why a BBMD gave no table, as text: no answer in time, or what failed.
pub fn unreachable_reason(error: &BvlcExchangeError) -> String {
    match error {
        BvlcExchangeError::NoReply(_) => NO_ANSWER.into(),
        other => other.to_string(),
    }
}

/// One BBMD as a survey left it: its BDT read (or why not) and what the
/// caller's read gave besides, which a skipped BBMD lacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Surveyed<X> {
    /// The BBMD.
    pub bbmd: SocketAddrV4,
    /// Its BDT, or why there is none.
    pub probe: Probe,
    /// What else the read gave (the TUI's is nothing, the MCP server's the
    /// FDT); `None` when the BBMD was not read.
    pub extra: Option<X>,
}

/// Read the BDT of each seed, then of every BBMD a table read lists,
/// breadth first, up to [`MAX_SURVEY`] reads; the rest come back
/// [`Outcome::Skipped`]. `read` gives each BBMD's BDT and anything else
/// the caller reads of it.
///
/// Up to [`SURVEY_AT_ONCE`] reads run at once, so silent peers wait out
/// their timeouts together. The result is the one reading them one at a
/// time gives: BBMDs are numbered in the order they are found, a read
/// starts only for one of the first [`MAX_SURVEY`], and a table's entries
/// join the list only once every read before it has been taken in, so a
/// fast answer from a later BBMD can't jump the breadth-first order.
pub async fn survey<X, F, R>(seeds: &[SocketAddrV4], read: F) -> Vec<Surveyed<X>>
where
    F: Fn(SocketAddrV4) -> R,
    R: Future<Output = (Probe, X)>,
{
    let read = &read;
    let mut seen = HashSet::new();
    // Every BBMD found, in breadth-first order.
    let mut found: Vec<SocketAddrV4> = seeds.iter().copied().filter(|s| seen.insert(*s)).collect();
    let mut done: Vec<Surveyed<X>> = Vec::new();
    // Reads finished ahead of an earlier one, by their place in `found`.
    let mut early = BTreeMap::new();
    let mut started = 0;
    let mut reads = FuturesUnordered::new();
    loop {
        while reads.len() < SURVEY_AT_ONCE && started < found.len().min(MAX_SURVEY) {
            let (index, bbmd) = (started, found[started]);
            reads.push(async move { (index, read(bbmd).await) });
            started += 1;
        }
        let Some((index, outcome)) = reads.next().await else {
            break;
        };
        early.insert(index, outcome);
        while let Some((probe, extra)) = early.remove(&done.len()) {
            if let Probe::Table(rows) = &probe {
                for row in rows {
                    if seen.insert(row.address) {
                        found.push(row.address);
                    }
                }
            }
            done.push(Surveyed {
                bbmd: found[done.len()],
                probe,
                extra: Some(extra),
            });
        }
    }
    let unread = found[done.len()..].iter();
    done.extend(unread.map(|&bbmd| Surveyed {
        bbmd,
        probe: Probe::Skipped,
        extra: None,
    }));
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(last: u8) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::new(10, 0, last, 1), 47808)
    }

    fn row(address: &str, mask: [u8; 4]) -> BdtRow {
        BdtRow {
            address: address.parse().unwrap(),
            mask: Ipv4Addr::from(mask),
        }
    }

    #[test]
    fn a_mask_says_how_peers_forward() {
        let unicast = row("10.0.1.1:47808", [255; 4]);
        assert!(unicast.is_unicast());
        assert_eq!(unicast.forward_to(), Ipv4Addr::new(10, 0, 1, 1));
        assert!(unicast.explain().contains("two-hop"));
        let directed = row("10.0.2.1:47808", [255, 255, 255, 0]);
        assert!(!directed.is_unicast());
        assert_eq!(directed.forward_to(), Ipv4Addr::new(10, 0, 2, 255));
        assert!(directed.explain().contains("10.0.2.255"));
    }

    #[test]
    fn an_entry_under_thirty_seconds_is_expiring() {
        let entry = |remaining| FdtRow {
            address: at(9),
            ttl: 60,
            remaining,
        };
        assert!(entry(29).expiring());
        assert!(!entry(30).expiring());
    }

    #[test]
    fn a_nak_and_a_timeout_sort_apart() {
        use bacnet_transport::bvll::BvlcFunction;
        let code = BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK;
        assert_eq!(probe_error(&BvlcExchangeError::Nak(code)), Probe::Nak(code));
        let silent = BvlcExchangeError::NoReply(Duration::from_secs(3));
        assert_eq!(probe_error(&silent), Probe::Unreachable("no answer".into()));
        // A success answers no read: not a refusal.
        let success = BvlcExchangeError::UnexpectedResponse {
            expected: BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
            got: BvlcFunction::BVLC_RESULT,
        };
        assert!(matches!(probe_error(&success), Probe::Unreachable(_)));
        let local = BvlcExchangeError::Transport(bacnet_types::error::Error::Encoding(
            "B/IP management client is stopped".into(),
        ));
        assert_eq!(
            probe_error(&local),
            Probe::Unreachable("encoding error: B/IP management client is stopped".into())
        );
        let mac = mac("10.0.0.1:47809".parse().unwrap());
        assert_eq!(address_of(&mac), Some("10.0.0.1:47809".parse().unwrap()));
        assert_eq!(address_of(&mac[..5]), None);
    }

    /// Two seeds are read first, in order, then the BBMDs their tables
    /// list; a seed listed by the other is read once.
    #[tokio::test]
    async fn every_seed_is_read_then_what_the_tables_list() {
        let tables = |bbmd: SocketAddrV4| match bbmd.ip().octets()[2] {
            1 => vec![at(1), at(2), at(3)],
            2 => vec![at(2), at(1)],
            _ => vec![],
        };
        let surveyed = survey(&[at(1), at(2), at(1)], |bbmd| async move {
            let rows = tables(bbmd)
                .into_iter()
                .map(|address| BdtRow {
                    address,
                    mask: Ipv4Addr::BROADCAST,
                })
                .collect();
            (Probe::Table(rows), bbmd.port())
        })
        .await;
        let order: Vec<SocketAddrV4> = surveyed.iter().map(|s| s.bbmd).collect();
        assert_eq!(order, [at(1), at(2), at(3)]);
        assert!(surveyed.iter().all(|s| s.extra == Some(47808)));
    }

    /// No seed reads nothing.
    #[tokio::test]
    async fn no_seed_reads_nothing() {
        let surveyed = survey(&[], |_| async { (Probe::Skipped, ()) }).await;
        assert!(surveyed.is_empty());
    }
}
