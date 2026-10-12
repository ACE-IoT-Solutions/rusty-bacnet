//! A BBMD's tables (Annex J.4.3, J.5.2).
//!
//! [`Bdt`] is the broadcast distribution table and [`Fdt`] the foreign
//! device table, each in an array whose size the caller picks. They read and
//! write through the codec's views: [`Bdt::set`] takes a
//! [`BdtEntries`](bacnet_bip_codec::BdtEntries), [`Bdt::entries`] gives one
//! for a Read-Broadcast-Distribution-Table-Ack, and [`Fdt::wire_entries`]
//! gives the rows of a Read-Foreign-Device-Table-Ack.
//!
//! The caller keeps everything else: the socket, access lists and rate
//! limits (through [`Fdt::register`]'s `admit`), counters, and persistence.
//! Registrations expire on the caller's clock: pass `now` to each call, and
//! call [`Fdt::purge`] now and then to free the slots of lapsed ones.
//!
//! ```
//! use core::net::{Ipv4Addr, SocketAddrV4};
//! use bacnet_bip_codec::{BdtEntries, BdtEntry};
//! use bacnet_bip_core::bbmd::{Bdt, Fdt, RegistrationLimits};
//! use bacnet_bip_core::Instant;
//!
//! let own = SocketAddrV4::new(Ipv4Addr::new(10, 0, 1, 1), 47808);
//! let peer = SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 1), 47808);
//! let mut bdt: Bdt<4> = Bdt::new(own);
//! // One peer, reached by one-hop distribution; the own row is added.
//! let rows = [BdtEntry::new(peer, Ipv4Addr::new(255, 255, 255, 0))];
//! bdt.set(BdtEntries::from_entries(&rows))?;
//! assert_eq!(bdt.len(), 2);
//!
//! let mut fdt: Fdt<4> = Fdt::new();
//! let device = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 7), 47808);
//! let now = Instant::from_secs(100);
//! let outcome = fdt.register(device, 60, now, &RegistrationLimits::default(), false, || true);
//! assert!(outcome.result.is_ok());
//! assert!(fdt.is_registered_at(device, now));
//! # Ok::<(), bacnet_bip_core::bbmd::BdtError>(())
//! ```

mod bdt;
mod fdt;

pub use bdt::{
    canonicalize, distribution_address, validate_entry, Bdt, BdtError, InvalidEntry, TWO_HOP_MASK,
};
pub use fdt::{
    Fdt, RegisterOutcome, Registered, Registration, RegistrationLimits, RegistrationRefusal,
    StorageFull, GRACE_PERIOD_SECS,
};
