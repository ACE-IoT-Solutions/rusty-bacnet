//! How a router is set up: its ports and its options.

use core::fmt;

use crate::table::{Convergence, Timing};
use crate::{Duration, Mac, PortId};

/// The longest NPDU a port carries, for each kind of send. B/IP and MS/TP
/// with extended frames carry 1,497 octets each way (Table 6-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// To one node.
    pub unicast: u16,
    /// To every node on the port.
    pub broadcast: u16,
}

impl Limits {
    /// The same limit both ways.
    pub const fn both(octets: u16) -> Self {
        Self {
            unicast: octets,
            broadcast: octets,
        }
    }

    /// The limit for a broadcast, or for a unicast.
    pub const fn get(self, broadcast: bool) -> usize {
        if broadcast {
            self.broadcast as usize
        } else {
            self.unicast as usize
        }
    }
}

/// One port as the router sees it: data, with no link behind it. The
/// caller drives the link and carries out the router's sends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortConfig {
    /// The network number. A port numbered 0 or 0xFFFF is accepted, as the
    /// hosted router accepts one, and gets no direct route.
    pub network: u16,
    /// The router's own MAC on the port.
    pub mac: Mac,
    /// The largest NPDU the port carries.
    pub max_npdu: Limits,
}

impl PortConfig {
    /// All zero bits: an unused slot.
    pub(super) const ZERO: Self = Self {
        network: 0,
        mac: Mac::EMPTY,
        max_npdu: Limits::both(0),
    };
}

/// Changes from the hosted router's behaviour that Clause 6 calls for, each
/// behind a switch. [`Fixes::NONE`], the default, behaves as the hosted
/// router does today; #1679 turns them on there once the hosted router runs
/// on this core. The rest of that list (R2, R4 to R7) comes with them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fixes {
    /// R1 (#1665): check every NPDU against its egress port's
    /// [`Limits`]. One routed toward a network draws reject reason 4
    /// naming that network; a port of a fan-out that can't carry it is left
    /// out and counted, with no reject. Off, the scratch is the only bound.
    pub egress_limits: bool,
    /// R3: at start, announce each port's network number with a
    /// Network-Number-Is before its I-Am-Router.
    pub announce_numbers: bool,
}

impl Fixes {
    /// Every switch off: today's behaviour.
    pub const NONE: Self = Self {
        egress_limits: false,
        announce_numbers: false,
    };

    /// Every switch on.
    pub const ALL: Self = Self {
        egress_limits: true,
        announce_numbers: true,
    };
}

/// A router's policy values and timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// How a claim that moves a route to another port is taken.
    pub convergence: Convergence,
    /// The most routes, direct ones included, that learning, management
    /// and I-Could-Be-Router offers may fill the table to. At most the
    /// table's capacity.
    pub route_cap: u16,
    /// How long a learned route lives without confirmation.
    pub learned_age: Duration,
    /// How often the sweep runs.
    pub sweep: Duration,
    /// The table's timers: the busy period, the reject hold-down and the
    /// corroboration window.
    pub timing: Timing,
    /// The least time between two searches for one network.
    pub discovery_cooldown: Duration,
    /// How long a search is remembered.
    pub discovery_age: Duration,
    /// Which fixes are on.
    pub fixes: Fixes,
}

impl Options {
    /// The hosted router's values: standard convergence, 256 routes, 300 s
    /// learned age, a 60 s sweep, [`Timing::DEFAULT`], a 5 s discovery
    /// cooldown and 30 s age, and no fixes.
    pub const HOSTED: Self = Self {
        convergence: Convergence::Standard,
        route_cap: 256,
        learned_age: Duration::from_secs(300),
        sweep: Duration::from_secs(60),
        timing: Timing::DEFAULT,
        discovery_cooldown: Duration::from_secs(5),
        discovery_age: Duration::from_secs(30),
        fixes: Fixes::NONE,
    };
}

impl Default for Options {
    fn default() -> Self {
        Self::HOSTED
    }
}

/// Why [`Router::new`](super::Router::new) refused its configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// No port at all.
    NoPorts,
    /// More ports than the storage holds, or than 255, the most a wire
    /// Port ID can name.
    TooManyPorts {
        /// Ports offered.
        count: usize,
        /// Ports the storage holds.
        capacity: usize,
    },
    /// Two ports share a network number.
    DuplicateNetwork(u16),
    /// A port's limit is under 64 octets or over the scratch.
    Limit(PortId),
    /// The route cap is over the table's capacity.
    RouteCap {
        /// The cap asked for.
        cap: u16,
        /// The routes the table holds.
        capacity: usize,
    },
    /// The table can't hold a route for every port.
    TableTooSmall,
    /// The sweep period is zero.
    ZeroSweep,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPorts => f.write_str("a router needs at least one port"),
            Self::TooManyPorts { count, capacity } => {
                write!(f, "{count} ports offered, room for {capacity}")
            }
            Self::DuplicateNetwork(network) => {
                write!(f, "Duplicate network number {network} in router ports")
            }
            Self::Limit(port) => write!(
                f,
                "port {} has an NPDU limit under 64 octets or over the scratch",
                port.0
            ),
            Self::RouteCap { cap, capacity } => {
                write!(f, "a route cap of {cap} is over the table's {capacity}")
            }
            Self::TableTooSmall => f.write_str("the table can't hold every port's network"),
            Self::ZeroSweep => f.write_str("the sweep period is zero"),
        }
    }
}

impl core::error::Error for ConfigError {}
