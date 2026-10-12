//! This transport's own management exchanges and the foreign-device
//! registration worker as they were before the node engine, kept verbatim
//! for the cutover's differential test (#1622): `client_management.rs`,
//! `bvlc_response.rs` and `foreign_device::run` as of the base. Only paths
//! changed, and the two snapshot helpers the engine's record no longer
//! needs are copied in.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bacnet_bip_codec::{BvlcFunction, BvlcResultCode, Bvll};
use bacnet_types::error::Error;
use bytes::BytesMut;
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::bbmd;
use crate::bip::client_snapshot::{
    BvlcClientSnapshot, BvlcRequestCounters, ForeignRegistrationOutcome,
};
use crate::bip::{BipSocket, BipTransport, ForeignDeviceConfig};
use crate::bvll::{decode_bip_mac, encode_bip_mac, encode_bvll, BvllMessage};

/// `BvlcClientSnapshot::counters` as of the base.
fn counters(snapshot: &mut BvlcClientSnapshot, function: BvlcFunction) -> &mut BvlcRequestCounters {
    match function {
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE => &mut snapshot.read_bdt,
        BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE => &mut snapshot.write_bdt,
        BvlcFunction::READ_FOREIGN_DEVICE_TABLE => &mut snapshot.read_fdt,
        BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY => &mut snapshot.delete_fdt_entry,
        BvlcFunction::REGISTER_FOREIGN_DEVICE => &mut snapshot.register_foreign_device,
        _ => unreachable!("only the five management helpers admit requests"),
    }
}

/// `client_snapshot::increment` as of the base.
fn increment(counter: &mut u64) {
    *counter = counter.saturating_add(1);
}

#[derive(Default)]
struct State {
    pending: Option<PendingBvlcResponse>,
    snapshot: BvlcClientSnapshot,
    next_attempt: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct ManagementClient {
    state: Mutex<State>,
}

impl ManagementClient {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(crate) fn snapshot(&self) -> BvlcClientSnapshot {
        let state = self.lock();
        let mut snapshot = state.snapshot;
        snapshot.foreign_registration.next_attempt_in = state
            .next_attempt
            .map(|when| when.saturating_duration_since(Instant::now()));
        snapshot
    }

    pub(crate) fn start(&self) {
        let mut state = self.lock();
        state.pending = None;
        state.next_attempt = None;
        state.snapshot.reset_status();
        state.snapshot.running = true;
    }

    pub(crate) fn stop(&self) {
        let mut state = self.lock();
        state.snapshot.running = false;
        state.pending = None;
        state.next_attempt = None;
        state.snapshot.reset_status();
    }

    pub(crate) fn schedule(&self, next: Instant) {
        let mut state = self.lock();
        if state.snapshot.running {
            state.next_attempt = Some(next);
        }
    }

    pub(crate) fn begin(
        self: &Arc<Self>,
        target: ([u8; 4], u16),
        function: BvlcFunction,
        expected: BvlcResponseKind,
        ttl: Option<u16>,
    ) -> Result<(RequestGuard, oneshot::Receiver<BvllMessage>), Error> {
        let mut state = self.lock();
        if !state.snapshot.running {
            return Err(Error::Encoding("B/IP management client is stopped".into()));
        }
        if state.pending.is_some() {
            increment(&mut counters(&mut state.snapshot, function).busy);
            return Err(Error::Encoding(
                "BVLC management request already in flight".into(),
            ));
        }
        // Retaining this allocation in the guard prevents identity reuse even
        // after a response releases the slot and a newer request acquires it.
        let owner = Arc::new(());
        let (tx, rx) = oneshot::channel();
        state.pending = Some(PendingBvlcResponse {
            owner: owner.clone(),
            target,
            function,
            expected,
            ttl,
            deadline: None,
            early_response: None,
            tx,
        });
        if let Some(ttl) = ttl {
            let foreign = &mut state.snapshot.foreign_registration;
            increment(&mut foreign.attempts);
            foreign.last_bbmd = Some(SocketAddrV4::new(Ipv4Addr::from(target.0), target.1));
            foreign.last_ttl = Some(ttl);
            foreign.last_result = None;
            foreign.last_outcome = Some(ForeignRegistrationOutcome::Pending);
        }
        Ok((
            RequestGuard {
                client: self.clone(),
                owner,
                function,
            },
            rx,
        ))
    }

    /// Deliver only a peer- and operation-matched response. Malformed Results
    /// cannot steal a request; typed malformed ACKs preserve helper decode errors.
    pub(crate) fn complete(&self, msg: &BvllMessage, sender: ([u8; 4], u16)) -> bool {
        let validity = match msg.function {
            BvlcFunction::BVLC_RESULT
            | BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK
            | BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK => match msg.message() {
                Ok(Bvll::ReadBroadcastDistributionTableAck(table)) => {
                    bbmd::bdt_from_wire(table).map(drop)
                }
                Ok(Bvll::ReadForeignDeviceTableAck(table)) => bbmd::fdt_from_wire(table).map(drop),
                Ok(_) => Ok(()),
                Err(error) => Err(error),
            },
            _ => return false,
        };
        let mut state = self.lock();
        if !state.snapshot.running {
            return false;
        }
        if validity.is_err() {
            increment(&mut state.snapshot.malformed_responses);
            if let Some(function) = state
                .pending
                .as_ref()
                .filter(|pending| {
                    pending.target == sender && pending.expected.accepts(msg.function)
                })
                .map(|pending| pending.function)
            {
                increment(&mut counters(&mut state.snapshot, function).malformed_responses);
            }
            if msg.function == BvlcFunction::BVLC_RESULT {
                return false;
            }
        }
        let matched = state.pending.as_ref().is_some_and(|pending| {
            pending.matches(sender, msg)
                && pending
                    .deadline
                    .is_none_or(|deadline| Instant::now() < deadline)
        });
        if !matched {
            if validity.is_ok() {
                increment(&mut state.snapshot.unmatched_responses);
                if let Ok(code) = decode_bvlc_result_code(msg) {
                    state.snapshot.last_unmatched_result = Some(code);
                }
            }
            return false;
        }
        let pending = state.pending.as_mut().expect("matched pending request");
        if pending.deadline.is_none() {
            // A fast receive worker can beat the sender's post-send accounting.
            // Keep one candidate, but count/deliver it only after local success.
            // A failed send or cancellation drops it with the owned slot.
            if pending.early_response.is_none() {
                pending.early_response = Some((msg.clone(), validity.is_ok()));
            }
            return true;
        }
        Self::deliver(&mut state, msg, validity.is_ok());
        true
    }

    fn deliver(state: &mut State, msg: &BvllMessage, valid_payload: bool) {
        let pending = state.pending.take().expect("matched pending request");
        if valid_payload {
            let counters = counters(&mut state.snapshot, pending.function);
            if msg.function == BvlcFunction::BVLC_RESULT {
                let code = decode_bvlc_result_code(msg).expect("validated result");
                increment(&mut counters.results);
                counters.last_result = Some(code);
                if pending.ttl.is_some() {
                    let foreign = &mut state.snapshot.foreign_registration;
                    foreign.last_result = Some(code);
                    foreign.last_outcome = Some(if code == BvlcResultCode::SUCCESSFUL_COMPLETION {
                        increment(&mut foreign.accepted);
                        ForeignRegistrationOutcome::Accepted
                    } else {
                        increment(&mut foreign.refused);
                        ForeignRegistrationOutcome::Refused
                    });
                }
            } else {
                increment(&mut counters.acknowledgements);
            }
        }
        let _ = pending.tx.send(msg.clone());
    }

    pub(crate) async fn request(
        self: &Arc<Self>,
        socket: &BipSocket,
        target: &[u8],
        request: Bvll<'_>,
        expected: BvlcResponseKind,
        timeout: Duration,
    ) -> Result<BvllMessage, Error> {
        let (ip, port) = decode_bip_mac(target)?;
        let ttl = match request {
            Bvll::RegisterForeignDevice { ttl } => Some(ttl),
            _ => None,
        };
        let (guard, rx) = self.begin((ip, port), request.function(), expected, ttl)?;
        let mut buf = BytesMut::new();
        if let Err(error) = encode_bvll(&mut buf, &request) {
            guard.fail(ForeignRegistrationOutcome::LocalError);
            return Err(error);
        }
        if let Err(error) = socket
            .send_to(&buf, SocketAddrV4::new(Ipv4Addr::from(ip), port))
            .await
        {
            guard.fail(ForeignRegistrationOutcome::LocalError);
            return Err(Error::Transport(error));
        }
        let deadline = guard.sent(timeout);
        match tokio::time::timeout_at(deadline, rx).await {
            Ok(Ok(msg)) => Ok(msg),
            Ok(Err(_)) => Err(Error::Encoding("BVLC response channel dropped".into())),
            Err(_) => {
                guard.fail(ForeignRegistrationOutcome::TimedOut);
                Err(Error::Timeout(timeout))
            }
        }
    }
}

pub(crate) struct RequestGuard {
    client: Arc<ManagementClient>,
    owner: Arc<()>,
    function: BvlcFunction,
}

impl RequestGuard {
    /// Called exactly once after UDP send succeeds. Publish that observation
    /// and any early matched response together under the same short lock.
    pub(crate) fn sent(&self, timeout: Duration) -> Instant {
        let deadline = Instant::now() + timeout;
        let mut state = self.client.lock();
        increment(&mut counters(&mut state.snapshot, self.function).sent);
        let early = state
            .pending
            .as_mut()
            .filter(|p| Arc::ptr_eq(&p.owner, &self.owner))
            .and_then(|pending| {
                pending.deadline = Some(deadline);
                pending.early_response.take()
            });
        if let Some((msg, valid_payload)) = early {
            ManagementClient::deliver(&mut state, &msg, valid_payload);
        }
        deadline
    }

    pub(crate) fn fail(&self, outcome: ForeignRegistrationOutcome) {
        let mut state = self.client.lock();
        if !state
            .pending
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(&p.owner, &self.owner))
        {
            return;
        }
        let pending = state.pending.take().expect("owned pending request");
        let counters = counters(&mut state.snapshot, self.function);
        match outcome {
            ForeignRegistrationOutcome::TimedOut => increment(&mut counters.timeouts),
            ForeignRegistrationOutcome::LocalError => increment(&mut counters.local_errors),
            _ => {}
        }
        if pending.ttl.is_some() {
            state.snapshot.foreign_registration.last_outcome = Some(outcome);
        }
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.fail(ForeignRegistrationOutcome::Cancelled);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BvlcResponseKind {
    Result,
    ReadBroadcastDistributionTableAck,
    ReadForeignDeviceTableAck,
}

impl BvlcResponseKind {
    pub(crate) fn accepts(self, function: BvlcFunction) -> bool {
        match self {
            Self::Result => function == BvlcFunction::BVLC_RESULT,
            Self::ReadBroadcastDistributionTableAck => {
                function == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK
                    || function == BvlcFunction::BVLC_RESULT
            }
            Self::ReadForeignDeviceTableAck => {
                function == BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK
                    || function == BvlcFunction::BVLC_RESULT
            }
        }
    }
}

pub(crate) struct PendingBvlcResponse {
    pub(crate) owner: Arc<()>,
    pub(crate) target: ([u8; 4], u16),
    pub(crate) function: BvlcFunction,
    pub(crate) expected: BvlcResponseKind,
    pub(crate) ttl: Option<u16>,
    pub(crate) deadline: Option<Instant>,
    /// At most one response received before local send completion; the bool
    /// records payload validity so accounting need not parse under its lock.
    pub(crate) early_response: Option<(BvllMessage, bool)>,
    pub(crate) tx: oneshot::Sender<BvllMessage>,
}

impl PendingBvlcResponse {
    pub(crate) fn matches(&self, sender: ([u8; 4], u16), msg: &BvllMessage) -> bool {
        if self.target != sender || !self.expected.accepts(msg.function) {
            return false;
        }
        if msg.function != BvlcFunction::BVLC_RESULT {
            return true;
        }
        let Ok(code) = decode_bvlc_result_code(msg) else {
            return false;
        };
        // A Result answers this request only with the request's own NAK, or
        // with success when no ACK is expected (Annex J.2.1).
        let Some(nak) = self.function.nak() else {
            return false;
        };
        code == nak
            || (self.expected == BvlcResponseKind::Result
                && code == BvlcResultCode::SUCCESSFUL_COMPLETION)
    }
}

/// The error for a response of the wrong function.
pub(crate) fn unexpected_response(expected: BvlcFunction, got: BvlcFunction) -> Error {
    Error::Encoding(format!("expected BVLC response {expected:?}, got {got:?}"))
}

/// The result code of a BVLC-Result, or why `msg` isn't a well-formed one.
pub(crate) fn decode_bvlc_result_code(msg: &BvllMessage) -> Result<BvlcResultCode, Error> {
    if msg.function != BvlcFunction::BVLC_RESULT {
        return Err(unexpected_response(BvlcFunction::BVLC_RESULT, msg.function));
    }
    match msg.message()? {
        Bvll::BvlcResult(code) => Ok(code),
        other => Err(unexpected_response(
            BvlcFunction::BVLC_RESULT,
            other.function(),
        )),
    }
}

/// `ForeignDeviceConfig::interval`, as of the base, which the cutover keeps.
pub(crate) fn interval(config: &ForeignDeviceConfig) -> Result<Duration, Error> {
    config.interval()
}

pub(crate) async fn run(
    socket: Arc<BipSocket>,
    management: Arc<ManagementClient>,
    config: ForeignDeviceConfig,
    interval: Duration,
) {
    let target = encode_bip_mac(config.bbmd_ip.octets(), config.bbmd_port);
    loop {
        // One attempt at a time, with no catch-up burst after a delayed poll.
        // Short intervals also bound response waiting, so a silent peer cannot
        // hold the slot for the full manual-request timeout on every renewal.
        let next = Instant::now() + interval;
        management.schedule(next);
        if let Err(error) = management
            .request(
                &socket,
                &target,
                Bvll::RegisterForeignDevice { ttl: config.ttl },
                BvlcResponseKind::Result,
                interval.min(BipTransport::BVLC_RESPONSE_TIMEOUT),
            )
            .await
        {
            tracing::debug!(bbmd = %SocketAddrV4::new(config.bbmd_ip, config.bbmd_port), %error,
                "Foreign registration attempt did not return a matched result");
        }
        tokio::time::sleep_until(next).await;
    }
}

/// `BipTransport::send_unicast` as of the base.
pub(crate) async fn send_unicast(socket: &BipSocket, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
    let (ip, port) = decode_bip_mac(mac)?;
    let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);

    let mut buf = BytesMut::with_capacity(4 + npdu.len());
    encode_bvll(&mut buf, &Bvll::OriginalUnicastNpdu(npdu))?;

    socket.send_to(&buf, dest).await.map_err(Error::Transport)?;

    Ok(())
}

/// `BipTransport::send_broadcast` as of the base, without a BBMD's forward
/// of its own broadcast, which the cutover leaves as it was.
pub(crate) async fn send_broadcast(
    socket: &BipSocket,
    foreign_device: Option<&ForeignDeviceConfig>,
    broadcast: SocketAddrV4,
    npdu: &[u8],
) -> Result<(), Error> {
    if let Some(fd) = foreign_device {
        let bbmd_addr = SocketAddrV4::new(fd.bbmd_ip, fd.bbmd_port);
        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        encode_bvll(&mut buf, &Bvll::DistributeBroadcastToNetwork(npdu))?;
        socket
            .send_to(&buf, bbmd_addr)
            .await
            .map_err(Error::Transport)?;
        return Ok(());
    }

    let dest = broadcast;

    let mut buf = BytesMut::with_capacity(4 + npdu.len());
    encode_bvll(&mut buf, &Bvll::OriginalBroadcastNpdu(npdu))?;

    socket.send_to(&buf, dest).await.map_err(Error::Transport)?;

    Ok(())
}
