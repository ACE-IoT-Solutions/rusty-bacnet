//! MS/TP egress admission for the embedded router.
//!
//! [`MstpEgress`] has no queue of its own.  A successful admission transfers
//! ownership directly to [`bacnet_mstp_core::MasterCore::enqueue_npdu`], which
//! copies the NPDU into the selected MS/TP queue.  This keeps backpressure
//! observable at the router boundary and avoids an unbounded adapter-side
//! staging path.

use bacnet_mstp_core::{Instant, MasterCore, NetworkPriority, QueueRejectReason, QueueStorage};
use bacnet_router_core::{
    AdmissionOutcome, Egress, EgressDestination, EgressRequest, LinkAddress, PortId,
};

/// The MS/TP link-layer broadcast station address.
pub const MSTP_BROADCAST: u8 = 0xff;

/// A synchronous MS/TP egress bound to one configured router port and time.
///
/// The timestamp is explicit so an ESP-IDF adapter can use the same monotonic
/// epoch as its owning MS/TP core.  Call [`Self::set_now`] before admitting a
/// later batch; the adapter never reads a platform clock implicitly.
pub struct MstpEgress<'core, 'rx, Q: QueueStorage> {
    core: &'core mut MasterCore<'rx, Q>,
    port: PortId,
    now: Instant,
}

impl<'core, 'rx, Q: QueueStorage> MstpEgress<'core, 'rx, Q> {
    /// Bind an MS/TP core to one router port and an explicit monotonic time.
    pub const fn new(core: &'core mut MasterCore<'rx, Q>, port: PortId, now: Instant) -> Self {
        Self { core, port, now }
    }

    /// Update the timestamp used for subsequent admissions.
    pub const fn set_now(&mut self, now: Instant) {
        self.now = now;
    }

    /// Return the bound router port.
    pub const fn port(&self) -> PortId {
        self.port
    }

    /// Return the timestamp used for subsequent admissions.
    pub const fn now(&self) -> Instant {
        self.now
    }

    /// Borrow the underlying MS/TP core after the adapter operation.
    pub fn core_mut(&mut self) -> &mut MasterCore<'rx, Q> {
        self.core
    }

    fn destination(destination: EgressDestination) -> Option<u8> {
        match destination {
            EgressDestination::Broadcast => Some(MSTP_BROADCAST),
            EgressDestination::Unicast(address) => Self::unicast_destination(address),
        }
    }

    fn unicast_destination(address: LinkAddress) -> Option<u8> {
        let bytes = address.as_slice();
        (bytes.len() == 1).then_some(bytes[0])
    }
}

impl<Q: QueueStorage> Egress for MstpEgress<'_, '_, Q> {
    fn try_admit(&mut self, request: EgressRequest<'_>) -> AdmissionOutcome {
        if request.port != self.port {
            return AdmissionOutcome::Invalid;
        }
        let Some(destination) = Self::destination(request.destination) else {
            return AdmissionOutcome::Invalid;
        };
        let Some(priority) = NetworkPriority::from_u8(request.priority) else {
            return AdmissionOutcome::Invalid;
        };

        match self
            .core
            .enqueue_npdu(destination, request.npdu, priority, self.now)
        {
            Ok(()) => AdmissionOutcome::AcceptedOwned,
            Err(QueueRejectReason::Full) => AdmissionOutcome::Full,
            Err(QueueRejectReason::Oversize) => AdmissionOutcome::Oversize,
            Err(QueueRejectReason::Expired) => AdmissionOutcome::Expired,
            // Starvation is not expected from enqueue_npdu, but preserving a
            // closed mapping keeps future queue policy changes from being
            // mistaken for successful ownership transfer.
            Err(QueueRejectReason::Starved) => AdmissionOutcome::Invalid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_mstp_codec::{decode_frame, Frame, FrameType};
    use bacnet_mstp_core::{Action, HeaplessQueues, MstpCoreConfig};

    type Queues = HeaplessQueues<1, 1>;

    fn config() -> MstpCoreConfig {
        let mut config = MstpCoreConfig::default();
        config.station = 1;
        config.max_master = 2;
        config
    }

    fn core<'rx>(rx: &'rx mut [u8]) -> MasterCore<'rx, Queues> {
        MasterCore::new(config(), Queues::new(), rx, Instant::ZERO).unwrap()
    }

    fn request<'a>(
        port: PortId,
        destination: EgressDestination,
        priority: u8,
        npdu: &'a [u8],
    ) -> EgressRequest<'a> {
        EgressRequest {
            port,
            destination,
            class: bacnet_router_core::TrafficClass::Data,
            priority,
            npdu,
        }
    }

    fn at(micros: u64) -> Instant {
        Instant::from_micros(micros)
    }

    #[test]
    fn admission_copies_owned_payload_and_preserves_priority() {
        let mut rx = [0; 1024];
        let mut core = core(&mut rx);
        let mut egress = MstpEgress::new(&mut core, PortId(7), at(1));
        let mut payload = [0x01, 0x02, 0x03];
        assert_eq!(
            egress.try_admit(request(
                PortId(7),
                EgressDestination::Unicast(LinkAddress::try_from_slice(&[2]).unwrap()),
                3,
                &payload,
            )),
            AdmissionOutcome::AcceptedOwned
        );
        payload.fill(0xff);
        assert_eq!(egress.core_mut().counters().unicast.depth, 1);

        // Give station 1 a token and let the queue scheduler select the
        // queued frame.  Decoding the emitted frame proves that the queue
        // owns the original bytes after the caller's buffer is reused.
        egress
            .core_mut()
            .on_validated_frame(Frame::new(FrameType::Token, 1, 2, &[]), at(2))
            .unwrap();
        egress.core_mut().on_deadline(at(50_000)).unwrap();
        let frame = loop {
            match egress.core_mut().poll_action() {
                Action::Transmit { frame, .. } => break frame,
                Action::Idle | Action::SetDirection(_) => {}
                _ => {}
            }
        };
        let (frame, _) = decode_frame(frame).unwrap();
        assert_eq!(frame.data, &[0x01, 0x02, 0x03]);
        assert_eq!(frame.header.destination, 2);
    }

    #[test]
    fn broadcast_maps_to_ff_and_one_byte_unicast_is_exact() {
        let mut rx = [0; 1024];
        let mut core = core(&mut rx);
        let mut egress = MstpEgress::new(&mut core, PortId(4), Instant::ZERO);
        assert_eq!(
            egress.try_admit(request(PortId(4), EgressDestination::Broadcast, 0, &[1])),
            AdmissionOutcome::AcceptedOwned
        );
        assert_eq!(
            egress.try_admit(request(
                PortId(4),
                EgressDestination::Unicast(LinkAddress::try_from_slice(&[3]).unwrap()),
                0,
                &[2],
            )),
            AdmissionOutcome::AcceptedOwned
        );
        assert_eq!(egress.core_mut().counters().broadcast.depth, 1);
        assert_eq!(egress.core_mut().counters().unicast.depth, 1);
    }

    #[test]
    fn full_oversize_expired_and_invalid_inputs_are_mapped() {
        let mut rx = [0; 1024];
        let mut core = core(&mut rx);
        let mut egress = MstpEgress::new(&mut core, PortId(2), at(10));
        let unicast = EgressDestination::Unicast(LinkAddress::try_from_slice(&[3]).unwrap());
        assert_eq!(
            egress.try_admit(request(PortId(2), unicast, 0, &[1])),
            AdmissionOutcome::AcceptedOwned
        );
        assert_eq!(
            egress.try_admit(request(PortId(2), unicast, 0, &[2])),
            AdmissionOutcome::Full
        );
        let oversized = [0; 502];
        egress.set_now(at(11));
        assert_eq!(
            egress.try_admit(request(
                PortId(2),
                EgressDestination::Broadcast,
                0,
                &oversized,
            )),
            AdmissionOutcome::Oversize
        );

        egress.set_now(at(9));
        assert_eq!(
            egress.try_admit(request(PortId(2), EgressDestination::Broadcast, 0, &[3])),
            AdmissionOutcome::Expired
        );
        egress.set_now(at(12));
        assert_eq!(
            egress.try_admit(request(PortId(99), EgressDestination::Broadcast, 0, &[4])),
            AdmissionOutcome::Invalid
        );
        let wide = LinkAddress::try_from_slice(&[1, 2]).unwrap();
        assert_eq!(
            egress.try_admit(request(
                PortId(2),
                EgressDestination::Unicast(wide),
                0,
                &[4],
            )),
            AdmissionOutcome::Invalid
        );
        assert_eq!(
            egress.try_admit(request(PortId(2), EgressDestination::Broadcast, 4, &[4])),
            AdmissionOutcome::Invalid
        );
    }

    #[test]
    fn direct_enqueue_does_not_add_a_second_queue() {
        let mut rx = [0; 1024];
        let mut core = core(&mut rx);
        let mut egress = MstpEgress::new(&mut core, PortId(5), at(1));
        assert_eq!(
            egress.try_admit(request(PortId(5), EgressDestination::Broadcast, 1, &[7])),
            AdmissionOutcome::AcceptedOwned
        );
        let counters = egress.core_mut().counters();
        assert_eq!(counters.broadcast.depth, 1);
        assert_eq!(counters.unicast.depth, 0);
    }
}
