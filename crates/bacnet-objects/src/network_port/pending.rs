//! Writes to a BBMD port's BBMD_Broadcast_Distribution_Table and
//! BBMD_Accept_FD_Registrations (Clause 12.56.34, 12.56.35, #1578).
//!
//! A write doesn't take effect at once: the value waits, Changes_Pending
//! reads TRUE, and a read of the property returns the value written, while
//! the BBMD keeps working with the old one, so a Read-BDT answers with the
//! BDT in use (Clause 12.56.13, 12.56.34). A ReinitializeDevice
//! ACTIVATE_CHANGES or WARMSTART puts the waiting values into effect
//! through the transport ([`NetworkPortObject::activate_changes`]).
//!
//! A BDT is checked against the transport's BDT policy when it is written,
//! so one the BBMD would refuse is refused then and changes nothing. The
//! rows take writes only when the port's owner answers ReinitializeDevice
//! (`set_activation_internal`); otherwise a written value could never take
//! effect, and the write is WRITE_ACCESS_DENIED.

use bacnet_types::bip_port::BbmdChanges;
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::bip_mode::{bdt_value, LiveMode};
use super::NetworkPortObject;
use crate::common;

impl NetworkPortObject {
    /// The value written to `property` and waiting for activation, if any.
    pub(super) fn pending_value(&self, property: P) -> Option<Result<PropertyValue, Error>> {
        match property {
            P::BBMD_BROADCAST_DISTRIBUTION_TABLE => self
                .pending
                .broadcast_distribution_table
                .as_deref()
                .map(bdt_value),
            P::BBMD_ACCEPT_FD_REGISTRATIONS => self
                .pending
                .accept_foreign_device_registrations
                .map(|accept| Ok(PropertyValue::Boolean(accept))),
            _ => None,
        }
    }

    /// Take a write to a property a BBMD port lets clients change; `None`
    /// for any other property, or a port not in BBMD mode.
    pub(super) fn write_pending(
        &mut self,
        property: P,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        let LiveMode::Bbmd(tables) = &self.mode else {
            return None;
        };
        // A value nothing could activate would wait for ever: without an
        // owner that answers ReinitializeDevice the rows stay read-only
        // (Clause 16.4 ties pending changes to ACTIVATE_CHANGES).
        if !self.activation && writable_pending_row(property) {
            return Some(Err(common::write_access_denied_error()));
        }
        match property {
            P::BBMD_BROADCAST_DISTRIBUTION_TABLE => Some((|| {
                if array_index.is_some() {
                    return Err(common::property_is_not_an_array_error());
                }
                let PropertyValue::ApplicationData(octets) = value else {
                    return Err(common::invalid_data_type_error());
                };
                let table = bacnet_encoding::constructed::decode_bdt_entry_list(octets)
                    .map_err(|_| common::invalid_data_encoding_error())?;
                tables.check_broadcast_distribution_table(&table)?;
                self.pending.broadcast_distribution_table = Some(table);
                Ok(())
            })()),
            P::BBMD_ACCEPT_FD_REGISTRATIONS => Some(match (array_index, value) {
                (Some(_), _) => Err(common::property_is_not_an_array_error()),
                (None, PropertyValue::Boolean(accept)) => {
                    self.pending.accept_foreign_device_registrations = Some(*accept);
                    Ok(())
                }
                _ => Err(common::invalid_data_type_error()),
            }),
            _ => None,
        }
    }

    /// Whether the pending values would still be taken, changing nothing:
    /// the check a ReinitializeDevice makes before its handler runs.
    pub fn check_pending_changes(&self) -> Result<(), Error> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let LiveMode::Bbmd(tables) = &self.mode else {
            return Err(common::write_access_denied_error());
        };
        match &self.pending.broadcast_distribution_table {
            Some(table) => tables.check_broadcast_distribution_table(table),
            // Any Boolean is a valid registration setting.
            None => Ok(()),
        }
    }

    /// Whether written values wait for activation: Changes_Pending.
    pub(super) fn changes_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Whether a ReinitializeDevice owner activates written values, so the
    /// pending-activation rows take writes.
    pub(super) fn activates_changes(&self) -> bool {
        self.activation
    }

    /// Put the values written to this port into effect, as a
    /// ReinitializeDevice ACTIVATE_CHANGES or WARMSTART does (Clause
    /// 12.56.13), and clear Changes_Pending. Whether there were any.
    ///
    /// The transport takes all of them or none: when it refuses, the error
    /// comes back and they stay pending.
    pub fn activate_changes(&mut self) -> Result<bool, Error> {
        if self.pending.is_empty() {
            return Ok(false);
        }
        match &self.mode {
            LiveMode::Bbmd(tables) => tables.activate(&self.pending)?,
            // A port whose mode changed dropped its pending values then.
            _ => return Err(common::write_access_denied_error()),
        }
        self.pending = BbmdChanges::default();
        Ok(true)
    }
}

/// The rows a write leaves pending for activation.
pub(super) fn writable_pending_row(property: P) -> bool {
    matches!(
        property,
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE | P::BBMD_ACCEPT_FD_REGISTRATIONS
    )
}
