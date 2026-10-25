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
//!   APDU, the priority and the expecting-reply bit back out of it, and the
//!   router's table picks the route and the next hop. The link MAC the
//!   stack sends to plays no part, so `router_mac` arguments are ignored. An
//!   NPDU with no DNET, which would be a local send on a link the
//!   application lacks, and network messages, which are the router's own
//!   business, are refused.
//! - **Answers** to the confirmed requests the stack received go back
//!   through each request's saved reply plan instead
//!   ([`application_replies`](super::application_replies)): from the
//!   endpoint that was asked, along the path the request came by.
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

use super::application_replies::Replies;
use super::dispatch::{Outputs, Shared};
use super::options::sealed::ApduSource;
use super::originate::{OriginateFailure, originate};
use super::{BACnetRouter, LocalApduReceiver, StartedRouter};
use crate::layer::ReceivedApdu;

/// The NPDUs the port holds for the stack above. Few: the port's task takes
/// an APDU off the router's local queue only once there is room here, so
/// the router's queue, with its admission accounting and, when tracked, its
/// per-source quota, stays the buffer that fills and drops under a slow
/// stack. Taking more would free the router's slots, and the quota with
/// them, for APDUs still waiting here.
const QUEUE: usize = 4;

/// Room for the network header in front of an APDU: what B/IP leaves
/// between its 1,497-octet NPDU and 1,476-octet APDU.
const NPCI_ROOM: u16 = (MAX_NPDU - 1476) as u16;

/// The application entity of a running [`BACnetRouter`] as a
/// [`TransportPort`] (#1808): give it to a stack that runs on a transport,
/// such as `bacnet-client`'s `BACnetClient`, and the stack's requests go
/// through the router's table, and its replies come back from the router.
///
/// Made by [`StartedRouter::into_application_port`], from the router's own
/// local APDU receiver: the stack on the port gets every APDU the router
/// delivers to its application. A remote device is addressed by its network
/// and MAC, as a routed device (with any MAC as the router's; the table picks
/// the next hop), whether its network is directly attached or not. A
/// device's reply carries its network and MAC as its source either way.
/// The port has no local network, so a send with no DNET, such as a local
/// broadcast, is refused with [`Error::Encoding`], and so is a network
/// message.
///
/// The stack's answers to confirmed requests it receives, such as the
/// acknowledgement of a confirmed COV notification, go back through the
/// reply plan the router saved with each request, for up to 30 seconds and
/// 64 requests at a time, as a server's replies do. A send the router
/// refuses there fails with the transport error the plan reports.
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
/// let (router, port, _controls) = started.into_application_port();
/// // `BACnetClient::generic_builder().transport(port).build()` gives a
/// // client whose requests the router originates.
/// # Ok(())
/// # }
/// ```
pub struct RouterApplicationPort {
    outputs: Weak<Outputs>,
    shared: Arc<Shared>,
    apdus: Option<ApduSource>,
    replies: Arc<Replies>,
    local_mac: MacAddr,
    max_npdu: NpduLimits,
    source: SourceSelection,
    task: Option<JoinHandle<()>>,
}

impl<A: LocalApduReceiver, C> StartedRouter<A, C> {
    /// The router, its application entity as a [`TransportPort`] fed by
    /// the router's own local APDU receiver, raw or tracked, and the
    /// network-control receiver, if one was asked for (#1808). See
    /// [`RouterApplicationPort`].
    ///
    /// The port reports the smallest NPDU limits of the router's ports, as
    /// they are now, and the longest of the router's MACs on them as its
    /// own: a request routed on from any port with the router's address as
    /// its source then fits.
    pub fn into_application_port(self) -> (BACnetRouter, RouterApplicationPort, Option<C>) {
        let port = self.router.application_port(self.apdus);
        (self.router, port, self.network_control)
    }
}

impl BACnetRouter {
    /// [`StartedRouter::into_application_port`], with `apdus`, which must
    /// be this router's own local receiver.
    pub(super) fn application_port<A: LocalApduReceiver>(&self, apdus: A) -> RouterApplicationPort {
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
            replies: Arc::default(),
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

    /// Send the NPDU a stack above wrote: an answer through its request's
    /// saved plan, anything else originated.
    async fn send(&self, npdu: &[u8]) -> Result<(), Error> {
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
        if let Some(response) = self.replies.plan_for(destination, npdu.payload) {
            return self
                .replies
                .send(&response, npdu.payload, npdu.expecting_reply, npdu.priority)
                .await;
        }
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

    /// How many reply plans the port keeps now.
    #[cfg(test)]
    pub(super) fn saved_replies(&self) -> usize {
        self.replies.len()
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
        let replies = Arc::clone(&self.replies);
        // No lock is held across an await: the only waits are the router's
        // local queue and room in the stack's. A slow stack holds up only
        // the router's local queue, which then drops new APDUs as it does
        // for any slow consumer; the router itself never waits.
        self.task = Some(tokio::spawn(async move {
            while let Some(apdu) = apdus.recv().await {
                let Some(npdu) = inbound(apdu, &replies) else {
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
        self.send(npdu).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send(npdu).await
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
/// above receives, with its origin as SNET and SADR, its reply plan kept in
/// `replies`; `None`, logged, when it has no origin that can be written.
fn inbound(apdu: ReceivedApdu, replies: &Replies) -> Option<ReceivedNpdu> {
    let (network, mac) = match (&apdu.source_network, apdu.ingress_network) {
        (Some(source), _) => (source.network, source.mac_address.as_slice()),
        (None, Some(network)) => (network, apdu.source_mac.as_slice()),
        (None, None) => {
            debug!("router application: an APDU with no ingress network");
            return None;
        }
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
    if let Some(response) = apdu.router_response {
        replies.save(network, mac, &apdu.apdu, response);
    }
    Some(ReceivedNpdu {
        npdu: Bytes::from(buf),
        source_mac: apdu.source_mac,
        link_layer_group: apdu.is_group && !apdu.global_broadcast,
        data_attributes: apdu.data_attributes,
        provenance: apdu.provenance,
        // Answers go back through the saved plan, not on the arrival
        // link's reply paths.
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
