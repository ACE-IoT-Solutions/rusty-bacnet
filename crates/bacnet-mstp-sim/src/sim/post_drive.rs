//! Physical release and transmit admission, independently of core completion.

use bacnet_mstp_core::TransmitId;

use super::{Event, Sim};
use crate::event::Class;
use crate::line::{Driver, TxId};
use crate::report::DriverRelease;

impl Sim {
    /// A release belongs to one monotonic line transmission ID, never just
    /// to a station. A power failure removes it, so an old queued release
    /// cannot clear a later transmission or re-enable an unpowered receiver.
    pub(super) fn release_driver(&mut self, tx: TxId, power_loss: bool) {
        let Some(owner) = self.line.release(tx) else {
            return;
        };
        self.meta.remove(&tx);
        self.report.driver_releases.push(DriverRelease {
            driver: owner.driver,
            start: owner.start,
            data_end: (self.now >= owner.data_end).then_some(owner.data_end),
            scheduled: owner.release,
            at: self.now,
            power_loss,
        });
        if let Driver::Station(station) = owner.driver {
            let node = self.index_of(station);
            if self.nodes[node].driving == Some(tx) {
                self.nodes[node].driving = None;
                if !power_loss {
                    self.nodes[node].listening_since = Some(self.now);
                }
            }
        }
    }

    /// Even when software completion already happened, admission cannot
    /// confirm a new start until this adapter's old physical driver releases.
    pub(super) fn wait_for_release(&mut self, node: usize, id: TransmitId) -> bool {
        let Some(owner) = self.nodes[node].driving.and_then(|tx| self.line.owner(tx)) else {
            return false;
        };
        if owner.release <= self.now {
            return false;
        }
        let release = owner.release;
        let station = self.nodes[node].station;
        let Some(master) = self.master_mut(node) else {
            return true;
        };
        if master
            .pending
            .as_ref()
            .is_some_and(|pending| pending.id == id)
        {
            let incarnation = master.incarnation;
            self.push(
                release,
                Class::AdapterStart,
                station,
                Event::AdapterStart {
                    node,
                    id,
                    incarnation,
                },
            );
        }
        true
    }
}
