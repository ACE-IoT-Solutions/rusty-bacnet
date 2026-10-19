//! The B/IP operating mode a transport lends the Network Port object.
//!
//! A B/IP port runs in NORMAL, FOREIGN or BBMD mode (Clause 12.56.21), and
//! which Network Port properties exist depends on the mode (Table 12-71,
//! footnotes 11 to 13, and the informative Table 12-72). The footnotes tie
//! the BBMD rows to a port able to act as a BBMD; this stack reads that as
//! a port configured as one, since a port's mode can't change while it is
//! registered. The transport layer owns that state and the object
//! layer serves it, and neither crate depends on the other, so the contract
//! between them lives here.
//!
//! A BBMD's tables change at run time, so they are lent as a live view,
//! [`BbmdTables`], which the object reads on each property read. Nothing is
//! copied, so the object cannot drift from the transport. The same view
//! checks a written BDT and puts written values into effect when they are
//! activated ([`BbmdChanges`]).
//!
//! The module exists only with the `std` feature. Both crates it joins are
//! hosted, and the shared `Arc` needs atomic compare-and-swap, which
//! bare-metal targets such as ARMv6-M lack (#1604).

use core::fmt;
use std::sync::Arc;

use crate::constructed::{BACnetBDTEntry, BACnetFDTEntry, BACnetHostNPort};
use crate::enums::{ErrorClass, ErrorCode, IPMode};
use crate::error::Error;

/// A live view of one BBMD's BDT and FDT.
///
/// Each call reports the tables as they stand at that moment. The Network
/// Port object calls these while a reader holds the server's object
/// database, so an implementation must answer promptly: no I/O and no
/// waiting on anything but a short critical section.
pub trait BbmdTables: Send + Sync {
    /// The BDT, this BBMD's own row included, in table order: the value of
    /// BBMD_Broadcast_Distribution_Table.
    fn broadcast_distribution_table(&self) -> Vec<BACnetBDTEntry>;

    /// Whether the BBMD accepts Register-Foreign-Device requests: the value
    /// of BBMD_Accept_FD_Registrations.
    fn accepts_foreign_device_registrations(&self) -> bool;

    /// The registrations that have not expired, each with the seconds it has
    /// left: the value of BBMD_Foreign_Device_Table.
    fn foreign_device_table(&self) -> Vec<BACnetFDTEntry>;

    /// Check a BDT written to BBMD_Broadcast_Distribution_Table, changing
    /// nothing: `Ok` when [`activate`](Self::activate) would take it. The
    /// default refuses every table, as for tables that can't be changed.
    fn check_broadcast_distribution_table(&self, table: &[BACnetBDTEntry]) -> Result<(), Error> {
        let _ = table;
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::WRITE_ACCESS_DENIED,
        })
    }

    /// Put written values into effect, as a ReinitializeDevice
    /// ACTIVATE_CHANGES or WARMSTART asks (Clause 12.56.13): all of them, or
    /// none and an error. The default refuses.
    fn activate(&self, changes: &BbmdChanges) -> Result<(), Error> {
        let _ = changes;
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::WRITE_ACCESS_DENIED,
        })
    }
}

/// Values written to a BBMD's Network Port properties and not yet in effect
/// (Clause 12.56.34, 12.56.35): `None` leaves that setting as it is.
///
/// It is `#[non_exhaustive]`, so the foreign-device rows can add their
/// settings: outside this crate, start from [`BbmdChanges::default`] and
/// set the fields that change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct BbmdChanges {
    /// A new BDT: BBMD_Broadcast_Distribution_Table.
    pub broadcast_distribution_table: Option<Vec<BACnetBDTEntry>>,
    /// Whether to accept foreign-device registrations:
    /// BBMD_Accept_FD_Registrations.
    pub accept_foreign_device_registrations: Option<bool>,
}

impl BbmdChanges {
    /// Whether nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.broadcast_distribution_table.is_none()
            && self.accept_foreign_device_registrations.is_none()
    }
}

/// A B/IP port's mode, with what that mode's Network Port properties need.
#[derive(Clone)]
pub enum BipPortMode {
    /// Plain B/IP: the port relays no broadcasts and joins no BBMD.
    Normal,
    /// The port joins a remote BBMD's network by registering with it.
    Foreign {
        /// The BBMD it registers with: FD_BBMD_Address.
        bbmd: BACnetHostNPort,
        /// The time to live, in seconds, it registers for:
        /// FD_Subscription_Lifetime.
        subscription_lifetime: u16,
    },
    /// The port relays broadcasts for its subnet as a BBMD.
    Bbmd {
        /// The BBMD's tables. `None` while a transport is configured but not
        /// yet started, since starting creates them.
        tables: Option<Arc<dyn BbmdTables>>,
    },
}

impl BipPortMode {
    /// The BACnet_IP_Mode value for this mode.
    pub fn ip_mode(&self) -> IPMode {
        match self {
            Self::Normal => IPMode::NORMAL,
            Self::Foreign { .. } => IPMode::FOREIGN,
            Self::Bbmd { .. } => IPMode::BBMD,
        }
    }
}

impl fmt::Debug for BipPortMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal => f.write_str("Normal"),
            Self::Foreign {
                bbmd,
                subscription_lifetime,
            } => f
                .debug_struct("Foreign")
                .field("bbmd", bbmd)
                .field("subscription_lifetime", subscription_lifetime)
                .finish(),
            Self::Bbmd { tables } => f
                .debug_struct("Bbmd")
                .field("started", &tables.is_some())
                .finish(),
        }
    }
}
