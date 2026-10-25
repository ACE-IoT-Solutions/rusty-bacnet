//! The router's application entity as a [`TransportPort`], so a stack built
//! for one link, such as `bacnet-client`'s `BACnetClient` with its
//! transaction state machine, runs on the router itself (#1808).
//!
//! # The seam
//!
//! A router's application is reachable at the router's address on each of
//! its networks (Clause 6.1), but it has no link of its own. The port gives
//! it one, with no wire behind it:
//!
//! - **Sends** go to [`BACnetRouter::originate`]. The stack above writes an
//!   NPDU as it would for any remote device: DNET and DADR, or a broadcast to
//!   one network or to all of them. The port reads the destination, the
//!   APDU, the priority and the expecting-reply bit back out of it, and the router's
//!   table picks the route and the next hop. The link MAC the stack sends
//!   to plays no part, so `router_mac` arguments are ignored. An NPDU with
//!   no DNET, which would be a local send on a link the application lacks,
//!   and network messages, which are the router's own business, are refused.
//! - **Receives** come from the router's local APDU receiver: every APDU the
//!   router delivers to its application is handed up as an NPDU whose SNET
//!   and SADR say where it came from. An APDU a router relayed keeps its
//!   SNET and SADR. One from a device on a directly attached network gets
//!   that port's network as its SNET and the device's link MAC as its SADR,
//!   which is the station the request was addressed to, so its reply
//!   completes the transaction keyed to the routed destination. The link
//!   source MAC is passed up unchanged.
//!
//! The router chooses the source address of each request
//! ([`SourceSelection`], the egress port's own by default), and its replies
//! reach the local receiver the way any APDU for the router does.

use std::io;
use std::sync::{Arc, Weak};

use bacnet_npdu::{Address, GLOBAL_NETWORK, MAX_NPDU, NetworkPriority, Npdu, NpduLimits};
use bacnet_router_core::router::{OriginateError, Request, SourceSelection};
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::MacAddr;
use bacnet_types::error::Error;
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::debug;

use super::dispatch::{Outputs, Shared};
use super::options::sealed::ApduSource;
use super::originate::{OriginateFailure, originate};
use super::{BACnetRouter, LocalApduReceiver};
use crate::layer::ReceivedApdu;

/// The NPDUs the port holds for the stack above, as many as the router's
/// local queue holds for the port.
const QUEUE: usize = 256;

/// Room for the network header in front of an APDU: what B/IP leaves
/// between its 1,497-octet NPDU and 1,476-octet APDU.
const NPCI_ROOM: u16 = (MAX_NPDU - 1476) as u16;

/// The application entity of a running [`BACnetRouter`] as a
/// [`TransportPort`] (#1808): give it to a stack that runs on a transport,
/// such as `bacnet-client`'s `BACnetClient`, and the stack's requests go
/// through the router's table, and its replies come back from the router.
///
/// Made by [`BACnetRouter::application_port`], which hands it the router's
/// local APDU receiver: the stack on the port gets every APDU the router
/// delivers to its application. A remote device is addressed by its network
/// and MAC, as a routed device (with any MAC as the router's; the table picks
/// the next hop), whether its network is directly attached or not. A
/// device's reply carries its network and MAC as its source either way.
/// The port has no local network, so a send with no DNET, such as a local
/// broadcast, is refused with [`Error::Encoding`], and so is a network
/// message.
///
/// A refusal from [`BACnetRouter::originate`] fails the send:
/// [`OriginateError::TooLong`] as [`Error::SendTooLong`], every other one
/// as [`Error::Transport`] carrying the [`OriginateFailure`], which
/// [`OriginateFailure::from_error`] gets back. A network the table lacks
/// fails with [`OriginateError::NoRoute`] and starts a search for it, as
/// `originate` does; once the router has learned the route, a retry goes.
///
/// [`TransportPort::stop`] ends the port's task. Stopping the router closes
/// its local receiver, which ends the task too, and every later send fails
/// with [`OriginateFailure::Stopped`].
///
/// ```no_run
/// # use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort};
/// # use bacnet_transport::loopback::LoopbackTransport;
/// # async fn run(ports: Vec<RouterPort<LoopbackTransport>>) -> Result<(), bacnet_types::error::Error> {
/// let started = BACnetRouter::start(ports, RouterOptions::new()).await?;
/// let port = started.router.application_port(started.apdus);
/// // `BACnetClient::generic_builder().transport(port).build()` gives a
/// // client whose requests the router originates.
/// # Ok(())
/// # }
/// ```
pub struct RouterApplicationPort {
    outputs: Weak<Outputs>,
    shared: Arc<Shared>,
    apdus: Option<ApduSource>,
    local_mac: MacAddr,
    max_npdu: NpduLimits,
    source: SourceSelection,
    task: Option<JoinHandle<()>>,
}

impl BACnetRouter {
    /// The router's application entity as a [`TransportPort`], fed by
    /// `apdus`, the local APDU receiver [`BACnetRouter::start`] returned,
    /// raw or tracked (#1808). See [`RouterApplicationPort`].
    ///
    /// The port reports the smallest NPDU limits of the router's ports, as
    /// they are now, and the longest of the router's MACs on them as its
    /// own: a request routed on from any port with the router's address as
    /// its source then fits.
    pub fn application_port<A: LocalApduReceiver>(&self, apdus: A) -> RouterApplicationPort {
        let router = self.shared.engine.lock();
        let ports = router.ports();
        // A started router has at least one port.
        let smallest = |limit: fn(&NpduLimits) -> u16| {
            ports
                .iter()
                .map(|port| limit(&port.max_npdu))
                .min()
                .unwrap_or(NpduLimits::STANDARD.unicast)
        };
        let max_npdu = NpduLimits {
            unicast: smallest(|limits| limits.unicast),
            broadcast: smallest(|limits| limits.broadcast),
        };
        let local_mac =
            ports
                .iter()
                .map(|port| port.mac.as_slice())
                .fold(&[][..], |longest, mac| {
                    if mac.len() > longest.len() {
                        mac
                    } else {
                        longest
                    }
                });
        let port = RouterApplicationPort {
            outputs: self.outputs.clone(),
            shared: Arc::clone(&self.shared),
            apdus: Some(apdus.into_source()),
            local_mac: MacAddr::from_slice(local_mac),
            max_npdu,
            source: SourceSelection::default(),
            task: None,
        };
        drop(router);
        port
    }
}

impl RouterApplicationPort {
    /// The router's address each request names as its source, by default
    /// [`SourceSelection::EgressPort`]: see [`Request::source`].
    #[must_use]
    pub fn with_source(mut self, source: SourceSelection) -> Self {
        self.source = source;
        self
    }

    /// Report `max_npdu` from [`TransportPort::max_npdu`] in place of the
    /// router ports' smallest limits. The router still refuses a request
    /// too long for the port it leaves by.
    #[must_use]
    pub fn with_max_npdu(mut self, max_npdu: NpduLimits) -> Self {
        self.max_npdu = max_npdu;
        self
    }

    /// Originate the NPDU a stack above wrote.
    fn send(&self, npdu: &[u8]) -> Result<(), Error> {
        let npdu = Npdu::decode(npdu)
            .map_err(|error| Error::Encoding(format!("router application send: {error}")))?;
        if npdu.message_type.is_some() {
            return Err(Error::Encoding(
                "the router's application port carries APDUs only; network messages are \
                 the router's own"
                    .into(),
            ));
        }
        let Some(destination) = npdu.destination else {
            return Err(Error::Encoding(
                "the router's application has no network of its own: address a device by \
                 its network, or broadcast to one network or to all"
                    .into(),
            ));
        };
        let mut request = Request::new(destination, npdu.payload);
        request.expecting_reply = npdu.expecting_reply;
        request.priority = npdu.priority;
        request.source = self.source;
        originate(&self.outputs, &self.shared, &request).map_err(failure_error)
    }

    fn end_task(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for RouterApplicationPort {
    fn drop(&mut self) {
        self.end_task();
    }
}

impl std::fmt::Debug for RouterApplicationPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouterApplicationPort")
            .field("local_mac", &self.local_mac)
            .field("max_npdu", &self.max_npdu)
            .field("source", &self.source)
            .field("started", &self.task.is_some())
            .finish_non_exhaustive()
    }
}

impl TransportPort for RouterApplicationPort {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let mut apdus = self.apdus.take().ok_or_else(|| {
            Error::Encoding("router application port already started".to_string())
        })?;
        let (tx, rx) = mpsc::channel(QUEUE);
        // No lock is held here: the only waits are the router's local queue
        // and room in the stack's, so a slow stack holds up only the
        // router's local queue, which then drops new APDUs as it does for
        // any slow consumer.
        self.task = Some(tokio::spawn(async move {
            while let Some(apdu) = apdus.recv().await {
                let Some(npdu) = inbound(apdu) else {
                    continue;
                };
                if tx.send(npdu).await.is_err() {
                    break;
                }
            }
        }));
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        if let Some(task) = self.task.take() {
            task.abort();
            // Cancelled, or ended on its own: either way it has stopped.
            let _ = task.await;
        }
        Ok(())
    }

    fn abort(&mut self) {
        self.end_task();
    }

    async fn send_unicast(&self, npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        self.send(npdu)
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send(npdu)
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        apdu_room(self.max_npdu.unicast)
    }

    fn egress_apdu_limit(&self) -> u16 {
        apdu_room(self.max_npdu.unicast)
    }

    fn max_npdu(&self) -> NpduLimits {
        self.max_npdu
    }
}

/// The APDU room in an NPDU of `max_npdu` octets, at most 1,476.
fn apdu_room(max_npdu: u16) -> u16 {
    max_npdu.saturating_sub(NPCI_ROOM).min(1476)
}

/// `apdu`, delivered to the router's application, as the NPDU the stack
/// above receives, with its origin as SNET and SADR; `None`, logged, when
/// it has none that can be written.
fn inbound(apdu: ReceivedApdu) -> Option<ReceivedNpdu> {
    let (network, mac) = match &apdu.source_network {
        Some(source) => (source.network, source.mac_address.as_slice()),
        None => (apdu.ingress_network?, apdu.source_mac.as_slice()),
    };
    let npdu = Npdu {
        expecting_reply: false,
        priority: NetworkPriority::Normal,
        destination: apdu.global_broadcast.then_some(Address {
            network: GLOBAL_NETWORK,
            mac: &[],
        }),
        source: Some(Address { network, mac }),
        hop_count: 255,
        message_type: None,
        vendor_id: None,
        payload: &apdu.apdu,
        payload_offset: 0,
    };
    let mut buf = vec![0; npdu.encoded_len()];
    if let Err(error) = npdu.encode_into(&mut buf) {
        debug!(%error, "router application: an APDU with no writable source");
        return None;
    }
    Some(ReceivedNpdu {
        npdu: Bytes::from(buf),
        source_mac: apdu.source_mac,
        link_layer_group: apdu.is_group && !apdu.global_broadcast,
        data_attributes: apdu.data_attributes,
        provenance: apdu.provenance,
        // The stack's replies go back through the router, routed, not on
        // the arrival link's reply paths.
        direct_response: None,
        reply_tx: None,
    })
}

/// The send error for an [`originate`] refusal.
fn failure_error(failure: OriginateFailure) -> Error {
    let kind = match failure {
        OriginateFailure::Refused(OriginateError::TooLong { octets, limit }) => {
            return Error::SendTooLong {
                length: octets,
                limit,
            };
        }
        OriginateFailure::Refused(OriginateError::NoRoute | OriginateError::Unreachable) => {
            io::ErrorKind::NetworkUnreachable
        }
        OriginateFailure::Refused(OriginateError::Busy) | OriginateFailure::QueueFull => {
            io::ErrorKind::WouldBlock
        }
        OriginateFailure::Stopped => io::ErrorKind::NotConnected,
        _ => io::ErrorKind::InvalidInput,
    };
    Error::Transport(io::Error::new(kind, failure))
}

impl OriginateFailure {
    /// The refusal behind `error`, when it is a send of a
    /// [`RouterApplicationPort`] that [`BACnetRouter::originate`] refused,
    /// other than for length ([`Error::SendTooLong`]).
    #[must_use]
    pub fn from_error(error: &Error) -> Option<Self> {
        match error {
            Error::Transport(error) => error.get_ref()?.downcast_ref::<Self>().copied(),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "application_tests.rs"]
mod tests;
