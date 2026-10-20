//! Values written to the BBMD's Network Port properties, checked against
//! the BDT policy and put into effect on activation (Clause 12.56.13,
//! 12.56.32 to 12.56.35, #1578, #940).
//!
//! The BDT and the NAT setting are checked and put into effect together:
//! behind a NAT router the BDT must list the global address and be
//! two-hop only, so a new global address and the table that lists it can
//! only go in at once.

use std::net::{IpAddr, SocketAddrV4};

use bacnet_bip_codec::{self as codec, BdtEntries};
use bacnet_types::bip_port::BbmdChanges;
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

use super::BbmdState;

/// The WriteProperty refusal of a value the BBMD can't take.
fn out_of_range() -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::VALUE_OUT_OF_RANGE,
    }
}

/// A written BDT as the rows the core takes. A row must name an IPv4 host
/// and carry a mask: no host is never a table entry (Clause 12.56.34), and
/// this BBMD resolves no host names.
fn rows(table: &[BACnetBDTEntry]) -> Result<Vec<codec::BdtEntry>, Error> {
    table
        .iter()
        .map(
            |entry| match (&entry.bbmd_address.host, entry.broadcast_mask) {
                (BACnetHostAddress::Ip(IpAddr::V4(ip)), Some(mask)) => Ok(codec::BdtEntry::new(
                    SocketAddrV4::new(*ip, entry.bbmd_address.port),
                    mask.into(),
                )),
                _ => Err(out_of_range()),
            },
        )
        .collect()
}

/// A written BACnet_IP_Global_Address: an IPv4 host and a nonzero port, or
/// the `None` host with port 0, which clears it (Clause 12.56.33). This
/// BBMD resolves no host names.
fn global_of(address: &BACnetHostNPort) -> Result<Option<SocketAddrV4>, Error> {
    match (&address.host, address.port) {
        (BACnetHostAddress::None, 0) => Ok(None),
        (BACnetHostAddress::Ip(IpAddr::V4(ip)), port) if port != 0 => {
            Ok(Some(SocketAddrV4::new(*ip, port)))
        }
        _ => Err(out_of_range()),
    }
}

/// The NAT setting and BDT `changes` leave a BBMD with.
struct Candidate {
    enabled: bool,
    global: Option<SocketAddrV4>,
    rows: Vec<codec::BdtEntry>,
}

impl BbmdState {
    /// Whether `table` would pass as this BBMD's BDT: the same checks
    /// [`Self::set_bdt`] makes, its own row included, on a copy.
    pub(crate) fn check_written_bdt(&self, table: &[BACnetBDTEntry]) -> Result<(), Error> {
        let mut changes = BbmdChanges::default();
        changes.broadcast_distribution_table = Some(table.to_vec());
        self.check_changes(&changes)
    }

    /// The NAT setting and BDT after `changes`, when they touch either.
    fn candidate(&self, changes: &BbmdChanges) -> Result<Option<Candidate>, Error> {
        if changes.broadcast_distribution_table.is_none()
            && changes.nat_traversal.is_none()
            && changes.global_address.is_none()
        {
            return Ok(None);
        }
        let rows = match &changes.broadcast_distribution_table {
            Some(table) => rows(table)?,
            None => self.tables.bdt.configured_rows().to_vec(),
        };
        let global = match &changes.global_address {
            Some(address) => global_of(address)?,
            None => self.nat.global,
        };
        Ok(Some(Candidate {
            enabled: changes.nat_traversal.unwrap_or(self.nat.enabled),
            global,
            rows,
        }))
    }

    /// Whether [`Self::activate`] would take `changes`, changing nothing:
    /// the BDT, the NAT setting and the global address are checked as they
    /// would go in together.
    pub(crate) fn check_changes(&self, changes: &BbmdChanges) -> Result<(), Error> {
        let Some(candidate) = self.candidate(changes)? else {
            return Ok(());
        };
        let nat = self
            .nat
            .core(candidate.enabled, candidate.global)
            .map_err(|_| out_of_range())?;
        let mut probe = self.tables.clone();
        probe
            .set_nat_and_bdt(nat, self.local, BdtEntries::from_entries(&candidate.rows))
            .map_err(|_| out_of_range())
    }

    /// Accept foreign-device registrations or stop. A policy that stops
    /// is kept, and comes back when registrations are accepted again; with
    /// none kept, [`ForeignDevicePolicy::default`](super::ForeignDevicePolicy::default) applies. Registrations
    /// already made stay until they lapse.
    pub fn set_accepts_foreign_device_registrations(&mut self, accept: bool) {
        match (accept, self.foreign_device_policy.is_some()) {
            (true, false) => {
                self.foreign_device_policy = Some(self.parked_policy.take().unwrap_or_default());
            }
            (false, true) => self.parked_policy = self.foreign_device_policy.take(),
            _ => {}
        }
    }

    /// Put `changes` into effect: the BDT and NAT setting together, then
    /// the registration setting, or none when the first are refused.
    pub(crate) fn activate(&mut self, changes: &BbmdChanges) -> Result<(), Error> {
        if let Some(candidate) = self.candidate(changes)? {
            self.apply_nat_and_bdt(candidate.enabled, candidate.global, &candidate.rows)
                .map_err(|_| out_of_range())?;
        }
        if let Some(accept) = changes.accept_foreign_device_registrations {
            self.set_accepts_foreign_device_registrations(accept);
        }
        Ok(())
    }
}
