//! Synchronous BACnet/IP UDP/BVLL adapter boundaries.
//!
//! This module deliberately contains no socket calls.  An ESP-IDF integration
//! supplies the datagram metadata to [`decode_ingress`] and implements
//! [`BipDatagramSink`] around its bounded UDP send path.  The adapter owns no
//! queue: a successful send means that the supplied sink copied or otherwise
//! took ownership of the complete BVLL datagram before returning.

use bacnet_mstp_core::Instant;
use bacnet_router_core::{
    decode_bip_datagram, encode_bvll, AdmissionOutcome, BipDecodeError, BipIngress, BvlcFunction,
    Egress, EgressDestination, EgressRequest, Ipv4SocketAddress, LinkAddress, PortId,
    UdpDatagramMeta,
};

/// A caller-provided timestamped UDP datagram sink.
///
/// Implementations normally copy `datagram` into a bounded socket/DMA
/// descriptor before returning [`AdmissionOutcome::AcceptedOwned`].  They must
/// not retain a borrow of `datagram` after the call returns.  The adapter does
/// not add a queue in front of this boundary.
pub trait BipDatagramSink {
    /// Try to synchronously own one complete BVLL datagram.
    fn try_send(
        &mut self,
        destination: Ipv4SocketAddress,
        datagram: &[u8],
        now: Instant,
        class: bacnet_router_core::TrafficClass,
    ) -> AdmissionOutcome;
}

/// Ingress facts plus the caller's monotonic receive timestamp.
///
/// [`BipIngress`] keeps the immediate UDP peer and logical Forwarded-NPDU
/// origin separate.  The timestamp is kept at the adapter boundary so a
/// router can apply one clock domain to admission, aging, and diagnostics
/// without the library calling an ESP-IDF clock itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BipIngressContext<'a> {
    /// Decoded BVLL and UDP provenance.
    pub ingress: BipIngress<'a>,
    /// Caller-supplied monotonic receive time.
    pub received_at: Instant,
}

/// Decode one complete UDP datagram with an explicit caller timestamp.
pub fn decode_ingress<'a>(
    datagram: &'a [u8],
    metadata: UdpDatagramMeta,
    received_at: Instant,
) -> Result<BipIngressContext<'a>, BipDecodeError> {
    Ok(BipIngressContext {
        ingress: decode_bip_datagram(datagram, metadata)?,
        received_at,
    })
}

/// Error returned when a link address cannot represent one B/IP socket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BipAddressError {
    /// B/IP link addresses are exactly four IPv4 octets followed by two port
    /// octets.  No shortening, padding, or truncation is performed.
    WrongLength {
        /// Number of bytes supplied by the caller.
        actual: usize,
    },
}

/// Convert an exactly six-byte B/IP link address to an IPv4 socket address.
pub fn socket_address_from_link(
    address: LinkAddress,
) -> Result<Ipv4SocketAddress, BipAddressError> {
    let bytes = address.as_slice();
    if bytes.len() != 6 {
        return Err(BipAddressError::WrongLength {
            actual: bytes.len(),
        });
    }
    Ok(Ipv4SocketAddress {
        ip: [bytes[0], bytes[1], bytes[2], bytes[3]],
        port: u16::from_be_bytes([bytes[4], bytes[5]]),
    })
}

/// Convert an IPv4 socket address to its exact six-byte B/IP link address.
pub fn link_address_from_socket(address: Ipv4SocketAddress) -> LinkAddress {
    let port = address.port.to_be_bytes();
    // This slice is statically six bytes and therefore cannot fail.
    LinkAddress::try_from_slice(&[
        address.ip[0],
        address.ip[1],
        address.ip[2],
        address.ip[3],
        port[0],
        port[1],
    ])
    .expect("six-byte B/IP link address always fits")
}

/// A bounded, synchronous B/IP egress adapter.
///
/// `scratch` is caller-owned BVLL storage.  It is used for exactly one send
/// attempt and is never retained as a backlog.  The concrete sink must own or
/// copy the bytes before reporting [`AdmissionOutcome::AcceptedOwned`].
pub struct BipEgress<'sink, 'scratch, S> {
    sink: &'sink mut S,
    scratch: &'scratch mut [u8],
    port: PortId,
    broadcast_destination: Ipv4SocketAddress,
    now: Instant,
    last_now: Instant,
}

impl<'sink, 'scratch, S> BipEgress<'sink, 'scratch, S>
where
    S: BipDatagramSink,
{
    /// Bind a sink, caller scratch buffer, router port, and IPv4 broadcast
    /// destination.  The broadcast destination is explicit so platform code
    /// can represent its configured subnet without hidden socket policy.
    pub const fn new(
        sink: &'sink mut S,
        scratch: &'scratch mut [u8],
        port: PortId,
        broadcast_destination: Ipv4SocketAddress,
        now: Instant,
    ) -> Self {
        Self {
            sink,
            scratch,
            port,
            broadcast_destination,
            now,
            last_now: now,
        }
    }

    /// Update the timestamp used for the next send attempt.
    pub const fn set_now(&mut self, now: Instant) {
        self.now = now;
    }

    /// Return the port to which this adapter is bound.
    pub const fn port(&self) -> PortId {
        self.port
    }

    /// Return the timestamp used for subsequent send attempts.
    pub const fn now(&self) -> Instant {
        self.now
    }

    /// Return the caller-provided BVLL scratch capacity.
    pub const fn scratch_capacity(&self) -> usize {
        self.scratch.len()
    }

    /// Borrow the concrete sink after an adapter operation.
    pub fn sink_mut(&mut self) -> &mut S {
        self.sink
    }

    fn destination(
        &self,
        destination: EgressDestination,
    ) -> Result<(Ipv4SocketAddress, BvlcFunction), BipAddressError> {
        match destination {
            EgressDestination::Broadcast => Ok((
                self.broadcast_destination,
                BvlcFunction::OriginalBroadcastNpdu,
            )),
            EgressDestination::Unicast(address) => Ok((
                socket_address_from_link(address)?,
                BvlcFunction::OriginalUnicastNpdu,
            )),
        }
    }
}

impl<S> Egress for BipEgress<'_, '_, S>
where
    S: BipDatagramSink,
{
    fn try_admit(&mut self, request: EgressRequest<'_>) -> AdmissionOutcome {
        if request.port != self.port {
            return AdmissionOutcome::Invalid;
        }
        let Ok((destination, function)) = self.destination(request.destination) else {
            return AdmissionOutcome::Invalid;
        };
        if self.now < self.last_now {
            return AdmissionOutcome::Expired;
        }

        // Observe the timestamp even when encoding or the real sink rejects
        // the item.  A caller cannot move this adapter's clock backward by
        // retrying after a failed admission.
        self.last_now = self.now;

        let length = match encode_bvll(self.scratch, function, request.npdu) {
            Ok(length) => length,
            Err(BipDecodeError::OutputTooSmall { .. } | BipDecodeError::FrameTooLong(_)) => {
                return AdmissionOutcome::Oversize;
            }
            Err(_) => return AdmissionOutcome::Invalid,
        };
        self.sink.try_send(
            destination,
            &self.scratch[..length],
            self.now,
            request.class,
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::drop_non_drop)]

    use super::*;
    use bacnet_router_core::{
        decode_bip_datagram, BipDecodeError, BvlcFunction, UdpDatagramMeta, BVLL_HEADER_LENGTH,
    };

    const PORT: PortId = PortId(3);
    const BROADCAST: Ipv4SocketAddress = Ipv4SocketAddress {
        ip: [192, 0, 2, 255],
        port: 47_808,
    };

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct RecordingSink {
        outcome: AdmissionOutcome,
        calls: usize,
        destination: Ipv4SocketAddress,
        frame: [u8; 64],
        length: usize,
        timestamp: Instant,
        class: bacnet_router_core::TrafficClass,
    }

    impl RecordingSink {
        const fn accepting() -> Self {
            Self {
                outcome: AdmissionOutcome::AcceptedOwned,
                calls: 0,
                destination: Ipv4SocketAddress {
                    ip: [0; 4],
                    port: 0,
                },
                frame: [0; 64],
                length: 0,
                timestamp: Instant::ZERO,
                class: bacnet_router_core::TrafficClass::Data,
            }
        }
    }

    impl BipDatagramSink for RecordingSink {
        fn try_send(
            &mut self,
            destination: Ipv4SocketAddress,
            datagram: &[u8],
            now: Instant,
            class: bacnet_router_core::TrafficClass,
        ) -> AdmissionOutcome {
            self.calls += 1;
            self.destination = destination;
            self.timestamp = now;
            self.class = class;
            if self.outcome == AdmissionOutcome::AcceptedOwned {
                self.frame[..datagram.len()].copy_from_slice(datagram);
                self.length = datagram.len();
            }
            self.outcome
        }
    }

    fn socket(ip: [u8; 4], port: u16) -> Ipv4SocketAddress {
        Ipv4SocketAddress { ip, port }
    }

    fn request<'a>(destination: EgressDestination, npdu: &'a [u8]) -> EgressRequest<'a> {
        EgressRequest {
            port: PORT,
            destination,
            class: bacnet_router_core::TrafficClass::Data,
            priority: 0,
            npdu,
        }
    }

    fn ingress_meta(group: bool) -> UdpDatagramMeta {
        UdpDatagramMeta {
            peer: socket([198, 51, 100, 10], 47_809),
            destination: Some(if group {
                [192, 0, 2, 255]
            } else {
                [192, 0, 2, 20]
            }),
            destination_is_group: group,
            truncated: false,
        }
    }

    #[test]
    fn socket_link_conversion_is_exactly_six_bytes() {
        let value = socket([203, 0, 113, 7], 0xbac1);
        let link = link_address_from_socket(value);
        assert_eq!(link.as_slice(), &[203, 0, 113, 7, 0xba, 0xc1]);
        assert_eq!(socket_address_from_link(link), Ok(value));

        for length in [0, 1, 5] {
            let address = LinkAddress::try_from_slice(&[0; 7][..length]).unwrap();
            assert_eq!(
                socket_address_from_link(address),
                Err(BipAddressError::WrongLength { actual: length })
            );
        }
        assert!(LinkAddress::try_from_slice(&[0; 7]).is_err());
    }

    #[test]
    fn outgoing_unicast_uses_exact_original_unicast_wire() {
        let mut sink = RecordingSink::accepting();
        let mut scratch = [0; 32];
        let mut egress = BipEgress::new(
            &mut sink,
            &mut scratch,
            PORT,
            BROADCAST,
            Instant::from_micros(11),
        );
        let destination = socket([198, 51, 100, 10], 47_809);
        let destination_link = link_address_from_socket(destination);
        let npdu = [0x01, 0x20, 0x00, 0xff];
        assert_eq!(
            egress.try_admit(request(EgressDestination::Unicast(destination_link), &npdu,)),
            AdmissionOutcome::AcceptedOwned
        );
        drop(egress);
        assert_eq!(sink.destination, destination);
        assert_eq!(sink.timestamp, Instant::from_micros(11));
        assert_eq!(sink.class, bacnet_router_core::TrafficClass::Data);
        assert_eq!(
            &sink.frame[..sink.length],
            &[0x81, 0x0a, 0x00, 0x08, 0x01, 0x20, 0x00, 0xff]
        );
        let decoded = decode_bip_datagram(
            &sink.frame[..sink.length],
            UdpDatagramMeta {
                peer: socket([192, 0, 2, 20], 47_808),
                destination: Some(destination.ip),
                destination_is_group: false,
                truncated: false,
            },
        )
        .unwrap();
        assert_eq!(decoded.function, BvlcFunction::OriginalUnicastNpdu);
        assert_eq!(decoded.npdu, npdu);
    }

    #[test]
    fn outgoing_broadcast_uses_exact_original_broadcast_wire() {
        let mut sink = RecordingSink::accepting();
        let mut scratch = [0; 32];
        let mut egress = BipEgress::new(&mut sink, &mut scratch, PORT, BROADCAST, Instant::ZERO);
        let npdu = [0x01, 0x00];
        assert_eq!(
            egress.try_admit(request(EgressDestination::Broadcast, &npdu)),
            AdmissionOutcome::AcceptedOwned
        );
        drop(egress);
        assert_eq!(sink.destination, BROADCAST);
        assert_eq!(&sink.frame[..sink.length], &[0x81, 0x0b, 0, 6, 1, 0]);
        let decoded = decode_bip_datagram(
            &sink.frame[..sink.length],
            UdpDatagramMeta {
                peer: socket([198, 51, 100, 10], 47_809),
                destination: Some(BROADCAST.ip),
                destination_is_group: true,
                truncated: false,
            },
        )
        .unwrap();
        assert_eq!(decoded.function, BvlcFunction::OriginalBroadcastNpdu);
        assert_eq!(decoded.npdu, npdu);
    }

    #[test]
    fn ingress_context_preserves_forwarded_origin_peer_and_timestamp() {
        let frame = [
            0x81, 0x04, 0x00, 0x0c, 203, 0, 113, 7, 0xba, 0xc1, 0x01, 0x00,
        ];
        let context =
            decode_ingress(&frame, ingress_meta(false), Instant::from_micros(44)).unwrap();
        assert_eq!(context.received_at, Instant::from_micros(44));
        assert_eq!(
            context.ingress.immediate_peer,
            socket([198, 51, 100, 10], 47_809)
        );
        assert_eq!(
            context.ingress.logical_source,
            socket([203, 0, 113, 7], 0xbac1)
        );
        assert_eq!(
            context.ingress.logical_source_mac().as_slice(),
            &[203, 0, 113, 7, 0xba, 0xc1]
        );
        assert_eq!(context.ingress.npdu, &[1, 0]);
    }

    #[test]
    fn ingress_fails_closed_for_destination_mismatch_missing_metadata_and_truncation() {
        let unicast = [0x81, 0x0a, 0, 6, 1, 0];
        assert_eq!(
            decode_ingress(&unicast, ingress_meta(true), Instant::ZERO),
            Err(BipDecodeError::DestinationClassMismatch)
        );
        let mut missing = ingress_meta(false);
        missing.destination = None;
        assert_eq!(
            decode_ingress(&unicast, missing, Instant::ZERO),
            Err(BipDecodeError::MissingDestinationMetadata)
        );
        let mut truncated = ingress_meta(false);
        truncated.truncated = true;
        assert_eq!(
            decode_ingress(&unicast, truncated, Instant::ZERO),
            Err(BipDecodeError::DatagramTruncated)
        );
        assert_eq!(
            decode_ingress(
                &[0x81, 0x0a, 0, 7, 1, 0],
                ingress_meta(false),
                Instant::ZERO
            ),
            Err(BipDecodeError::LengthMismatch {
                declared: 7,
                actual: 6
            })
        );
    }

    #[test]
    fn sink_outcomes_are_returned_without_an_adapter_backlog() {
        for outcome in [
            AdmissionOutcome::Full,
            AdmissionOutcome::Oversize,
            AdmissionOutcome::PortDown,
            AdmissionOutcome::Expired,
            AdmissionOutcome::Invalid,
        ] {
            let mut sink = RecordingSink {
                outcome,
                ..RecordingSink::accepting()
            };
            let mut scratch = [0; 32];
            let mut egress = BipEgress::new(
                &mut sink,
                &mut scratch,
                PORT,
                BROADCAST,
                Instant::from_micros(7),
            );
            assert_eq!(
                egress.try_admit(request(EgressDestination::Broadcast, &[1, 2])),
                outcome
            );
            drop(egress);
            assert_eq!(sink.calls, 1);
        }
    }

    #[test]
    fn traffic_class_reaches_sink_for_reserved_control_admission() {
        struct ReservedSink;

        impl BipDatagramSink for ReservedSink {
            fn try_send(
                &mut self,
                _destination: Ipv4SocketAddress,
                _datagram: &[u8],
                _now: Instant,
                class: bacnet_router_core::TrafficClass,
            ) -> AdmissionOutcome {
                match class {
                    bacnet_router_core::TrafficClass::Data => AdmissionOutcome::Full,
                    bacnet_router_core::TrafficClass::NetworkControl => {
                        AdmissionOutcome::AcceptedOwned
                    }
                }
            }
        }

        let mut sink = ReservedSink;
        let mut scratch = [0; 32];
        let mut egress = BipEgress::new(&mut sink, &mut scratch, PORT, BROADCAST, Instant::ZERO);
        assert_eq!(
            egress.try_admit(request(EgressDestination::Broadcast, &[1])),
            AdmissionOutcome::Full
        );
        let mut control = request(EgressDestination::Broadcast, &[2]);
        control.class = bacnet_router_core::TrafficClass::NetworkControl;
        assert_eq!(egress.try_admit(control), AdmissionOutcome::AcceptedOwned);
    }

    #[test]
    fn local_validation_and_scratch_limits_fail_before_sink() {
        let mut sink = RecordingSink::accepting();
        let mut scratch = [0xa5; BVLL_HEADER_LENGTH + 1];
        let mut egress = BipEgress::new(
            &mut sink,
            &mut scratch,
            PORT,
            BROADCAST,
            Instant::from_micros(10),
        );
        assert_eq!(
            egress.try_admit(request(EgressDestination::Broadcast, &[1, 2])),
            AdmissionOutcome::Oversize
        );
        drop(egress);
        assert_eq!(sink.calls, 0);
        assert_eq!(scratch, [0xa5; BVLL_HEADER_LENGTH + 1]);

        let wide = LinkAddress::try_from_slice(&[1, 2]).unwrap();
        let mut egress = BipEgress::new(
            &mut sink,
            &mut scratch,
            PORT,
            BROADCAST,
            Instant::from_micros(10),
        );
        assert_eq!(
            egress.try_admit(request(EgressDestination::Unicast(wide), &[1])),
            AdmissionOutcome::Invalid
        );
        assert_eq!(
            egress.try_admit(EgressRequest {
                port: PortId(99),
                ..request(EgressDestination::Broadcast, &[1])
            }),
            AdmissionOutcome::Invalid
        );
        egress.set_now(Instant::from_micros(9));
        assert_eq!(
            egress.try_admit(request(EgressDestination::Broadcast, &[1])),
            AdmissionOutcome::Expired
        );
        drop(egress);
        assert_eq!(sink.calls, 0);
    }
}
