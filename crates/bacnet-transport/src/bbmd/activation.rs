//! Values written to the BBMD's Network Port properties, checked against
//! the BDT policy and put into effect on activation (Clause 12.56.13,
//! 12.56.34, 12.56.35, #1578).

use std::net::{IpAddr, SocketAddrV4};

use bacnet_bip_codec::{self as codec, BdtEntries};
use bacnet_types::bip_port::BbmdChanges;
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostAddress};
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

impl BbmdState {
    /// Whether `table` would pass as this BBMD's BDT: the same checks
    /// [`Self::set_bdt`] makes, its own row included, on a copy.
    pub(crate) fn check_written_bdt(&self, table: &[BACnetBDTEntry]) -> Result<(), Error> {
        let rows = rows(table)?;
        let mut probe = self.tables.bdt.clone();
        probe
            .set(BdtEntries::from_entries(&rows))
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

    /// Put `changes` into effect: the BDT, then the registration setting,
    /// or neither when the BDT is refused.
    pub(crate) fn activate(&mut self, changes: &BbmdChanges) -> Result<(), Error> {
        if let Some(table) = &changes.broadcast_distribution_table {
            let rows = rows(table)?;
            self.tables
                .bdt
                .set(BdtEntries::from_entries(&rows))
                .map_err(|_| out_of_range())?;
        }
        if let Some(accept) = changes.accept_foreign_device_registrations {
            self.set_accepts_foreign_device_registrations(accept);
        }
        Ok(())
    }
}
