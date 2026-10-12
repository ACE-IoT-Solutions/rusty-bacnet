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
/// Maximum NPDU payload in a standard MS/TP frame.
pub const STANDARD_MSTP_MAX_NPDU: usize = 501;
/// Maximum NPDU payload in an extended DER/DNER frame.
pub const EXTENDED_MSTP_MAX_NPDU: usize = 1497;
/// Maximum encoded storage required for a standard frame.
pub const STANDARD_MSTP_FRAME_STORAGE: usize = 511;
/// Maximum encoded storage required for an extended frame.
pub const EXTENDED_MSTP_FRAME_STORAGE: usize = 1516;

/// The largest NPDU a remote station is known to accept.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MstpPeerCapability {
    /// The peer accepts only standard MS/TP frames.
    Standard,
    /// The peer accepts extended DER/DNER frames.
    Extended,
}

impl MstpPeerCapability {
    /// Return the peer's advertised/verified NPDU ceiling.
    pub const fn max_npdu_len(self) -> usize {
        match self {
            Self::Standard => STANDARD_MSTP_MAX_NPDU,
            Self::Extended => EXTENDED_MSTP_MAX_NPDU,
        }
    }
}

/// A synchronous MS/TP egress bound to one configured router port and time.
///
/// The timestamp is explicit so an ESP-IDF adapter can use the same monotonic
/// epoch as its owning MS/TP core.  Call [`Self::set_now`] before admitting a
/// later batch; the adapter never reads a platform clock implicitly.
pub struct MstpEgress<'core, 'rx, Q: QueueStorage, const NPDU: usize = STANDARD_MSTP_MAX_NPDU> {
    core: &'core mut MasterCore<'rx, Q, NPDU>,
    port: PortId,
    now: Instant,
    /// The greatest timestamp offered to this adapter so far.
    ///
    /// `MasterCore::enqueue_npdu` intentionally does not advance its event
    /// clock: queue admission is also used by callers that have not yet
    /// delivered a bus event.  Keep this boundary-local watermark so a clock
    /// regression cannot be mistaken for a fresh admission.
    last_now: Instant,
    peer_capability: MstpPeerCapability,
}

/// Adapter bound to the standard 501-byte MS/TP storage profile.
pub type StandardMstpEgress<'core, 'rx, Q> = MstpEgress<'core, 'rx, Q, STANDARD_MSTP_MAX_NPDU>;

/// Adapter bound to the extended DER/DNER storage profile.
pub type ExtendedMstpEgress<'core, 'rx, Q> = MstpEgress<'core, 'rx, Q, EXTENDED_MSTP_MAX_NPDU>;

impl<'core, 'rx, Q: QueueStorage, const NPDU: usize> MstpEgress<'core, 'rx, Q, NPDU> {
    /// Bind an MS/TP core to one router port and an explicit monotonic time.
    pub const fn new(
        core: &'core mut MasterCore<'rx, Q, NPDU>,
        port: PortId,
        now: Instant,
    ) -> Self {
        Self {
            core,
            port,
            now,
            last_now: now,
            // Extended operation is opt-in.  A segment remains safe for
            // legacy stations until the owning application verifies peer
            // capability explicitly.
            peer_capability: MstpPeerCapability::Standard,
        }
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
    pub fn core_mut(&mut self) -> &mut MasterCore<'rx, Q, NPDU> {
        self.core
    }

    /// Return the NPDU payload ceiling of this MS/TP storage profile.
    pub const fn max_npdu_len(&self) -> usize {
        NPDU
    }

    /// Whether this adapter uses the extended DER/DNER storage profile.
    pub const fn is_extended(&self) -> bool {
        NPDU == EXTENDED_MSTP_MAX_NPDU
    }

    /// Set the verified capability of stations on this MS/TP segment.
    ///
    /// The default is [`MstpPeerCapability::Standard`], including for an
    /// extended local core.  This fail-closed default prevents an extended
    /// NPDU from being emitted toward a mixed or unverified segment.
    pub const fn set_peer_capability(&mut self, capability: MstpPeerCapability) {
        self.peer_capability = capability;
    }

    /// Return the currently configured remote-station capability.
    pub const fn peer_capability(&self) -> MstpPeerCapability {
        self.peer_capability
    }

    fn destination(destination: EgressDestination) -> Option<u8> {
        match destination {
            EgressDestination::Broadcast => Some(MSTP_BROADCAST),
            EgressDestination::Unicast(address) => Self::unicast_destination(address),
        }
    }

    fn unicast_destination(address: LinkAddress) -> Option<u8> {
        match address.as_slice() {
            [destination] if *destination != MSTP_BROADCAST => Some(*destination),
            _ => None,
        }
    }
}

impl<Q: QueueStorage, const NPDU: usize> Egress for MstpEgress<'_, '_, Q, NPDU> {
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
        if request.npdu.len() > self.peer_capability.max_npdu_len() {
            return AdmissionOutcome::Oversize;
        }
        if self.now < self.last_now {
            return AdmissionOutcome::Expired;
        }
        // Advance the adapter watermark even when the concrete queue rejects
        // the item.  Time is observed at this boundary independently of queue
        // capacity, and a later call must not regress behind it.
        self.last_now = self.now;

        // Upstream's unified queue owns both application and network-layer
        // NPDUs. The router still classifies control traffic for diagnostics,
        // but admission and backpressure use the core's single bounded API.
        let admitted = self
            .core
            .enqueue_npdu(destination, request.npdu, priority, self.now);
        match admitted {
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
    use bacnet_mstp_codec::{decode_frame, decode_frame_in_place, Frame, FrameType};
    use bacnet_mstp_core::{
        Action, ActionBuffers, ExtendedActionBuffers, HeaplessQueues, MasterCore, MstpCoreConfig,
    };
    use bacnet_router_core::TrafficClass;

    type Queues = HeaplessQueues<1, 1>;
    type Actions = ActionBuffers<1>;
    type ExtendedQueues = HeaplessQueues<1, 1, EXTENDED_MSTP_MAX_NPDU>;
    type ExtendedActions = ExtendedActionBuffers<1>;

    fn config() -> MstpCoreConfig {
        MstpCoreConfig {
            station: 1,
            max_master: 2,
            ..MstpCoreConfig::default()
        }
    }

    fn core<'rx>(rx: &'rx mut [u8], actions: &'rx mut Actions) -> MasterCore<'rx, Queues> {
        MasterCore::new(config(), Queues::new(), rx, actions, Instant::ZERO).unwrap()
    }

    fn extended_core<'rx>(
        rx: &'rx mut [u8],
        actions: &'rx mut ExtendedActions,
    ) -> MasterCore<'rx, ExtendedQueues, EXTENDED_MSTP_MAX_NPDU> {
        extended_core_at_station(rx, actions, 1)
    }

    fn extended_core_at_station<'rx>(
        rx: &'rx mut [u8],
        actions: &'rx mut ExtendedActions,
        station: u8,
    ) -> MasterCore<'rx, ExtendedQueues, EXTENDED_MSTP_MAX_NPDU> {
        let mut config = config();
        config.station = station;
        let config = config.with_extended_frames();
        MasterCore::new(config, ExtendedQueues::new(), rx, actions, Instant::ZERO).unwrap()
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
        let mut actions = Actions::new();
        let mut core = core(&mut rx, &mut actions);
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
        let mut actions = Actions::new();
        let mut core = core(&mut rx, &mut actions);
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
        let mut actions = Actions::new();
        let mut core = core(&mut rx, &mut actions);
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
        for invalid in [
            LinkAddress::default(),
            LinkAddress::try_from_slice(&[0xff]).unwrap(),
        ] {
            assert_eq!(
                egress.try_admit(request(
                    PortId(2),
                    EgressDestination::Unicast(invalid),
                    0,
                    &[4],
                )),
                AdmissionOutcome::Invalid
            );
        }
        assert_eq!(
            egress.try_admit(request(PortId(2), EgressDestination::Broadcast, 4, &[4])),
            AdmissionOutcome::Invalid
        );
    }

    #[test]
    fn direct_enqueue_does_not_add_a_second_queue() {
        let mut rx = [0; 1024];
        let mut actions = Actions::new();
        let mut core = core(&mut rx, &mut actions);
        let mut egress = MstpEgress::new(&mut core, PortId(5), at(1));
        assert_eq!(
            egress.try_admit(request(PortId(5), EgressDestination::Broadcast, 1, &[7])),
            AdmissionOutcome::AcceptedOwned
        );
        let counters = egress.core_mut().counters();
        assert_eq!(counters.broadcast.depth, 1);
        assert_eq!(counters.unicast.depth, 0);
    }

    #[test]
    fn saturated_queue_backpressures_network_control() {
        let mut rx = [0; 1024];
        let mut actions = Actions::new();
        let mut core = core(&mut rx, &mut actions);
        let mut egress = MstpEgress::new(&mut core, PortId(5), at(1));
        let destination = EgressDestination::Unicast(LinkAddress::try_from_slice(&[2]).unwrap());

        assert_eq!(
            egress.try_admit(request(PortId(5), destination, 0, &[0x10])),
            AdmissionOutcome::AcceptedOwned
        );
        assert_eq!(
            egress.try_admit(request(PortId(5), destination, 0, &[0x11])),
            AdmissionOutcome::Full
        );

        let mut control = request(PortId(5), destination, 1, &[0x20]);
        control.class = TrafficClass::NetworkControl;
        assert_eq!(egress.try_admit(control), AdmissionOutcome::Full);
        let counters = egress.core_mut().counters();
        assert_eq!(counters.unicast.depth, 1);
    }

    #[test]
    fn extended_profile_admits_1497_bytes_and_emits_decodable_extended_frame() {
        let mut rx = [0; 4096];
        let mut actions = ExtendedActions::new();
        let mut core = extended_core(&mut rx, &mut actions);
        let mut egress = ExtendedMstpEgress::new(&mut core, PortId(9), at(1));
        assert_eq!(egress.max_npdu_len(), EXTENDED_MSTP_MAX_NPDU);
        assert!(egress.is_extended());
        assert_eq!(egress.peer_capability(), MstpPeerCapability::Standard);
        egress.set_peer_capability(MstpPeerCapability::Extended);

        let payload = [0xa5; EXTENDED_MSTP_MAX_NPDU];
        let destination = EgressDestination::Unicast(LinkAddress::try_from_slice(&[2]).unwrap());
        assert_eq!(
            egress.try_admit(EgressRequest {
                port: PortId(9),
                destination,
                class: TrafficClass::Data,
                priority: 0,
                npdu: &payload,
            }),
            AdmissionOutcome::AcceptedOwned
        );

        egress
            .core_mut()
            .on_validated_frame(Frame::new(FrameType::Token, 1, 2, &[]), at(2))
            .unwrap();
        egress.core_mut().on_deadline(at(50_000)).unwrap();
        let frame = loop {
            match egress.core_mut().poll_action() {
                Action::Transmit { frame, .. } => break frame,
                Action::Idle | Action::SetDirection(_) => {}
                other => panic!("unexpected action: {other:?}"),
            }
        };
        let frame_len = frame.len();
        let mut encoded = [0; EXTENDED_MSTP_FRAME_STORAGE];
        encoded[..frame_len].copy_from_slice(frame);
        let (decoded, consumed) = decode_frame_in_place(&mut encoded[..frame_len]).unwrap();
        assert_eq!(consumed, frame_len);
        assert_eq!(decoded.header.destination, 2);
        assert_eq!(decoded.data.len(), EXTENDED_MSTP_MAX_NPDU);
        assert_eq!(decoded.data, payload);

        let mut receiver_rx = [0; 4096];
        let mut receiver_actions = ExtendedActions::new();
        let mut receiver = extended_core_at_station(&mut receiver_rx, &mut receiver_actions, 2);
        receiver.on_validated_frame(decoded, at(3)).unwrap();
        match receiver.poll_action() {
            Action::DeliverNpdu {
                source,
                destination,
                npdu,
                ..
            } => {
                assert_eq!(source, 1);
                assert_eq!(destination, 2);
                assert_eq!(npdu, payload);
            }
            other => panic!("expected extended delivery, got {other:?}"),
        }
    }

    #[test]
    fn extended_profile_fails_closed_for_unverified_standard_peer() {
        let mut rx = [0; 4096];
        let mut actions = ExtendedActions::new();
        let mut core = extended_core(&mut rx, &mut actions);
        let mut egress = ExtendedMstpEgress::new(&mut core, PortId(9), at(1));
        let payload = [0xa5; STANDARD_MSTP_MAX_NPDU + 1];
        assert_eq!(
            egress.try_admit(EgressRequest {
                port: PortId(9),
                destination: EgressDestination::Broadcast,
                class: TrafficClass::Data,
                priority: 0,
                npdu: &payload,
            }),
            AdmissionOutcome::Oversize
        );
        assert_eq!(egress.core_mut().counters().broadcast.depth, 0);
    }

    #[test]
    fn standard_profile_rejects_502_bytes_before_queue_mutation() {
        let mut rx = [0; 1024];
        let mut actions = Actions::new();
        let mut core = core(&mut rx, &mut actions);
        let mut egress = StandardMstpEgress::new(&mut core, PortId(9), at(1));
        let payload = [0xa5; STANDARD_MSTP_MAX_NPDU + 1];
        assert_eq!(
            egress.try_admit(EgressRequest {
                port: PortId(9),
                destination: EgressDestination::Broadcast,
                class: TrafficClass::Data,
                priority: 0,
                npdu: &payload,
            }),
            AdmissionOutcome::Oversize
        );
        assert_eq!(egress.core_mut().counters().broadcast.depth, 0);
    }

    #[test]
    fn extended_profile_admits_the_first_extended_boundary() {
        let mut rx = [0; 4096];
        let mut actions = ExtendedActions::new();
        let mut core = extended_core(&mut rx, &mut actions);
        let mut egress = ExtendedMstpEgress::new(&mut core, PortId(9), at(1));
        egress.set_peer_capability(MstpPeerCapability::Extended);
        let payload = [0x5a; STANDARD_MSTP_MAX_NPDU + 1];
        assert_eq!(
            egress.try_admit(EgressRequest {
                port: PortId(9),
                destination: EgressDestination::Unicast(
                    LinkAddress::try_from_slice(&[2]).unwrap(),
                ),
                class: TrafficClass::Data,
                priority: 0,
                npdu: &payload,
            }),
            AdmissionOutcome::AcceptedOwned
        );
        egress
            .core_mut()
            .on_validated_frame(Frame::new(FrameType::Token, 1, 2, &[]), at(2))
            .unwrap();
        egress.core_mut().on_deadline(at(50_000)).unwrap();
        let frame = loop {
            match egress.core_mut().poll_action() {
                Action::Transmit { frame, .. } => break frame,
                Action::Idle | Action::SetDirection(_) => {}
                other => panic!("unexpected action: {other:?}"),
            }
        };
        let frame_len = frame.len();
        let mut encoded = [0; EXTENDED_MSTP_FRAME_STORAGE];
        encoded[..frame_len].copy_from_slice(frame);
        let (decoded, _) = decode_frame_in_place(&mut encoded[..frame_len]).unwrap();
        assert_eq!(decoded.data, payload);

        let mut receiver_rx = [0; 4096];
        let mut receiver_actions = ExtendedActions::new();
        let mut receiver = extended_core_at_station(&mut receiver_rx, &mut receiver_actions, 2);
        receiver.on_validated_frame(decoded, at(3)).unwrap();
        match receiver.poll_action() {
            Action::DeliverNpdu { npdu, .. } => assert_eq!(npdu, payload),
            other => panic!("expected extended delivery, got {other:?}"),
        }
    }
}
