//! Optional, bounded APDU diagnostics at the network-layer boundary.
//!
//! Publishing is synchronous and callback-free: the network receive path only
//! decodes a snapshot and sends it into Tokio's bounded broadcast ring. Slow
//! consumers observe [`broadcast::error::RecvError::Lagged`] without applying
//! backpressure to packet admission or dispatch.

use std::sync::{Arc, Mutex};

use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_encoding::npdu::{decode_npdu, NpduAddress};
use bacnet_types::{error::Error, MacAddr};
use bytes::Bytes;
use tokio::sync::broadcast;

/// Maximum observer ring capacity accepted by high-level bindings.
pub const MAX_APDU_OBSERVER_CAPACITY: usize = 65_536;

/// Packet direction at the local [`crate::layer::NetworkLayer`] boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApduDirection {
    /// NPDU accepted from a transport.
    Inbound,
    /// NPDU submitted to a transport.
    Outbound,
}

/// Decode stage that rejected an observed packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeStage {
    /// Network-layer header or payload framing.
    Npdu,
    /// Application-layer PDU framing.
    Apdu,
}

/// Stable, deliberately small APDU summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApduSummary {
    /// Stable snake-case name for the APDU PDU type.
    pub pdu_type: &'static str,
    /// Raw confirmed or unconfirmed service choice, when the PDU carries one.
    pub service_choice: Option<u32>,
    /// Invoke identifier, when the PDU carries one.
    pub invoke_id: Option<u8>,
}

/// Typed diagnostic decode failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError {
    /// Protocol layer at which decoding failed.
    pub stage: DecodeStage,
    /// Stable broad error category suitable for metrics and bindings.
    pub category: &'static str,
    /// Human-readable decoder detail.
    pub message: String,
}

/// Decode outcome for an observed packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApduDecode {
    /// NPDU and APDU framing decoded successfully.
    Decoded(ApduSummary),
    /// NPDU or APDU framing failed to decode.
    Error(DecodeError),
}

/// One immutable observer event. `raw_npdu` is the exact network-layer bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApduObserverEvent {
    /// Whether the event was observed entering or leaving the network layer.
    pub direction: ApduDirection,
    /// The data-link peer that delivered or will receive the NPDU.
    pub immediate_peer: MacAddr,
    /// Origin claimed by a BACnet/IP Forwarded-NPDU, which is not authenticated.
    pub claimed_forwarded_origin: Option<MacAddr>,
    /// Routed source for inbound traffic or routed destination for outbound traffic.
    pub routed_address: Option<NpduAddress>,
    /// Exact encoded NPDU bytes observed at the boundary.
    pub raw_npdu: Bytes,
    /// Projected APDU summary or decode failure.
    pub decode: ApduDecode,
}

/// Cloneable publisher for a bounded APDU event ring.
///
/// The optional sender allows [`Self::close`] to wake all receivers while
/// clones of the observer still exist. No user callback runs in [`Self::publish`].
#[derive(Clone)]
pub struct ApduObserver {
    sender: Arc<Mutex<Option<broadcast::Sender<ApduObserverEvent>>>>,
}

impl ApduObserver {
    /// Create a bounded observer and startup-safe initial receiver.
    ///
    /// A capacity of zero is promoted to one and capacities above
    /// [`MAX_APDU_OBSERVER_CAPACITY`] are capped.
    pub fn new(capacity: usize) -> (Self, broadcast::Receiver<ApduObserverEvent>) {
        let (sender, receiver) = broadcast::channel(capacity.clamp(1, MAX_APDU_OBSERVER_CAPACITY));
        (
            Self {
                sender: Arc::new(Mutex::new(Some(sender))),
            },
            receiver,
        )
    }

    /// Subscribe to events published after this call, unless the observer is closed.
    pub fn subscribe(&self) -> Option<broadcast::Receiver<ApduObserverEvent>> {
        self.sender
            .lock()
            .ok()?
            .as_ref()
            .map(broadcast::Sender::subscribe)
    }

    /// Close all receivers without waiting for them.
    pub fn close(&self) {
        if let Ok(mut sender) = self.sender.lock() {
            sender.take();
        }
    }

    /// Publish without awaiting a consumer or executing user code.
    pub(crate) fn publish(&self, event: ApduObserverEvent) {
        if let Ok(sender) = self.sender.lock() {
            if let Some(sender) = sender.as_ref() {
                let _ = sender.send(event);
            }
        }
    }
}

/// Decode an exact NPDU snapshot into a diagnostic event.
///
/// Decode errors are projected into the event and never escape into the packet
/// admission path.
pub(crate) fn decode_npdu_event(
    direction: ApduDirection,
    immediate_peer: MacAddr,
    claimed_forwarded_origin: Option<MacAddr>,
    raw_npdu: Bytes,
) -> ApduObserverEvent {
    match decode_npdu(raw_npdu.clone()) {
        Ok(npdu) => {
            let routed_address = match direction {
                ApduDirection::Inbound => npdu.source.clone(),
                ApduDirection::Outbound => npdu.destination.clone(),
            };
            let decode = match decode_apdu(npdu.payload) {
                Ok(apdu) => ApduDecode::Decoded(summary(&apdu)),
                Err(error) => ApduDecode::Error(decode_error(DecodeStage::Apdu, &error)),
            };
            ApduObserverEvent {
                direction,
                immediate_peer,
                claimed_forwarded_origin,
                routed_address,
                raw_npdu,
                decode,
            }
        }
        Err(error) => ApduObserverEvent {
            direction,
            immediate_peer,
            claimed_forwarded_origin,
            routed_address: None,
            raw_npdu,
            decode: ApduDecode::Error(decode_error(DecodeStage::Npdu, &error)),
        },
    }
}

fn summary(apdu: &Apdu) -> ApduSummary {
    let (pdu_type, service_choice, invoke_id) = match apdu {
        Apdu::ConfirmedRequest(pdu) => (
            "confirmed_request",
            Some(pdu.service_choice.to_raw() as u32),
            Some(pdu.invoke_id),
        ),
        Apdu::UnconfirmedRequest(pdu) => (
            "unconfirmed_request",
            Some(pdu.service_choice.to_raw() as u32),
            None,
        ),
        Apdu::SimpleAck(pdu) => (
            "simple_ack",
            Some(pdu.service_choice.to_raw() as u32),
            Some(pdu.invoke_id),
        ),
        Apdu::ComplexAck(pdu) => (
            "complex_ack",
            Some(pdu.service_choice.to_raw() as u32),
            Some(pdu.invoke_id),
        ),
        Apdu::SegmentAck(pdu) => ("segment_ack", None, Some(pdu.invoke_id)),
        Apdu::Error(pdu) => (
            "error",
            Some(pdu.service_choice.to_raw() as u32),
            Some(pdu.invoke_id),
        ),
        Apdu::Reject(pdu) => ("reject", None, Some(pdu.invoke_id)),
        Apdu::Abort(pdu) => ("abort", None, Some(pdu.invoke_id)),
    };
    ApduSummary {
        pdu_type,
        service_choice,
        invoke_id,
    }
}

fn decode_error(stage: DecodeStage, error: &Error) -> DecodeError {
    let category = match error {
        Error::Decoding { .. } => "decoding",
        Error::BufferTooShort { .. } => "buffer-too-short",
        Error::InvalidTag(_) => "invalid-tag",
        Error::OutOfRange(_) => "out-of-range",
        Error::Encoding(_) => "encoding",
        _ => "other",
    };
    DecodeError {
        stage,
        category,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_encoding::npdu::{encode_npdu, Npdu};
    use bytes::BytesMut;

    fn event(raw_npdu: &'static [u8]) -> ApduObserverEvent {
        decode_npdu_event(
            ApduDirection::Inbound,
            MacAddr::from_slice(&[1]),
            None,
            Bytes::from_static(raw_npdu),
        )
    }

    #[tokio::test]
    async fn bounded_ring_reports_exact_lag_and_keeps_latest_event() {
        let (observer, mut receiver) = ApduObserver::new(1);
        observer.publish(event(&[0xff]));
        observer.publish(event(&[0xfe]));

        assert!(matches!(
            receiver.recv().await,
            Err(broadcast::error::RecvError::Lagged(1))
        ));
        assert_eq!(
            receiver.recv().await.unwrap().raw_npdu,
            Bytes::from_static(&[0xfe])
        );
    }

    #[tokio::test]
    async fn zero_capacity_is_promoted_and_close_wakes_every_receiver() {
        let (observer, mut initial) = ApduObserver::new(0);
        let mut subscriber = observer.subscribe().expect("observer is open");

        observer.close();

        assert_eq!(
            initial.recv().await,
            Err(broadcast::error::RecvError::Closed)
        );
        assert_eq!(
            subscriber.recv().await,
            Err(broadcast::error::RecvError::Closed)
        );
        assert!(observer.subscribe().is_none());
    }

    #[test]
    fn malformed_npdu_and_apdu_are_projected_without_panicking() {
        let malformed_npdu = event(&[0xff]);
        assert!(matches!(
            malformed_npdu.decode,
            ApduDecode::Error(DecodeError {
                stage: DecodeStage::Npdu,
                ..
            })
        ));

        let mut encoded = BytesMut::new();
        encode_npdu(
            &mut encoded,
            &Npdu {
                payload: Bytes::from_static(&[0x10]),
                ..Npdu::default()
            },
        )
        .unwrap();
        let malformed_apdu = decode_npdu_event(
            ApduDirection::Inbound,
            MacAddr::new(),
            None,
            encoded.freeze(),
        );
        assert!(matches!(
            malformed_apdu.decode,
            ApduDecode::Error(DecodeError {
                stage: DecodeStage::Apdu,
                ..
            })
        ));
    }

    #[test]
    fn decoded_event_preserves_raw_bytes_routing_and_summary() {
        let routed_source = NpduAddress {
            network: 42,
            mac_address: MacAddr::from_slice(&[7, 8]),
        };
        let mut encoded = BytesMut::new();
        encode_npdu(
            &mut encoded,
            &Npdu {
                source: Some(routed_source.clone()),
                payload: Bytes::from_static(&[0x10, 0x08]),
                ..Npdu::default()
            },
        )
        .unwrap();
        let raw_npdu = encoded.freeze();

        let decoded = decode_npdu_event(
            ApduDirection::Inbound,
            MacAddr::from_slice(&[9]),
            Some(MacAddr::from_slice(&[10])),
            raw_npdu.clone(),
        );

        assert_eq!(decoded.raw_npdu, raw_npdu);
        assert_eq!(decoded.routed_address, Some(routed_source));
        assert_eq!(
            decoded.claimed_forwarded_origin,
            Some(MacAddr::from_slice(&[10]))
        );
        assert!(matches!(
            decoded.decode,
            ApduDecode::Decoded(ApduSummary {
                pdu_type: "unconfirmed_request",
                service_choice: Some(8),
                invoke_id: None,
            })
        ));
    }
}
