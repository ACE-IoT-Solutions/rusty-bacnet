//! The two sides of the cutover's differential test (#1622): the receive
//! path, management exchanges and registration worker as they were
//! ([`super::legacy`]), and the node engine's, each on a recording socket
//! under paused Tokio time, behind one interface the test drives.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bacnet_bip_codec::{BdtEntries, BvlcFunction, Bvll};
use bacnet_bip_core::node::{Link, Mode};
use bacnet_bip_core::LocalSubnet;
use bacnet_types::error::Error;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::client_snapshot::{BvlcClientSnapshot, ClientRecord, ForeignRegistrationOutcome};
use super::exchange::{decode_bvlc_result_code, unexpected_response, Exchange};
use super::ingress::{handle_datagram, Arrival, IngressAddresses};
use super::io::RecvContext;
use super::legacy::client::{self as old, BvlcResponseKind, ManagementClient, RequestGuard};
use super::legacy::receive as old_receive;
use super::node::{NodeHandle, Outcome};
use super::rate_limit::ManagementRateLimiter;
use super::receive_counters::{ReceiveCounters, ReceiveDrops};
use super::send_sink::SendSink;
use super::{BbmdState, BdtEntry, BipSocket, ForeignDeviceConfig, ForeignDevicePolicy};
use crate::bbmd;
use crate::bvll::{encode_bip_mac, BvllMessage};
use crate::port::ReceivedNpdu;
use crate::udp_metadata::ReceivedDatagram;

pub(super) const OWN_IP: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 10);
pub(super) const SUBNET_BROADCAST: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 255);
/// Another of the host's addresses, which a wildcard bind takes unicast to.
pub(super) const OTHER_LOCAL: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 11);
pub(super) const LISTENER_INTERFACE: u32 = 2;
/// The manual requests' timeout: off the 10 ms grid the registration timers
/// keep, so no request ends at the instant a timer fires.
pub(super) const MANUAL_TIMEOUT: Duration = Duration::from_millis(3002);
/// The one row a Write-BDT carries.
const WRITE_ROW: [u8; 10] = [10, 0, 0, 1, 0xBA, 0xC0, 255, 255, 255, 255];

/// How the node takes part in the network.
#[derive(Clone, Debug)]
pub(super) enum ModeConfig {
    Normal,
    Foreign(ForeignDeviceConfig),
    Bbmd,
}

/// A node's configuration, the same on both sides.
#[derive(Clone, Debug)]
pub(super) struct Config {
    pub(super) own_port: u16,
    pub(super) broadcast: Ipv4Addr,
    pub(super) wildcard: bool,
    /// Per-address mode, with broadcast listeners on interface 2.
    pub(super) listener: bool,
    pub(super) mode: ModeConfig,
}

impl Config {
    pub(super) fn own(&self) -> SocketAddrV4 {
        SocketAddrV4::new(OWN_IP, self.own_port)
    }

    fn broadcast_address(&self) -> SocketAddrV4 {
        SocketAddrV4::new(self.broadcast, self.own_port)
    }

    fn unicast_ips(&self) -> Vec<Ipv4Addr> {
        if self.wildcard {
            vec![OWN_IP, OTHER_LOCAL]
        } else {
            vec![OWN_IP]
        }
    }

    pub(super) fn foreign(&self) -> Option<&ForeignDeviceConfig> {
        match &self.mode {
            ModeConfig::Foreign(foreign) => Some(foreign),
            _ => None,
        }
    }

    /// A BBMD's state: its own row, a remote peer and a remote subnet,
    /// taking foreign devices.
    fn bbmd_state(&self) -> Option<Arc<Mutex<BbmdState>>> {
        if !matches!(self.mode, ModeConfig::Bbmd) {
            return None;
        }
        let mut state = BbmdState::new(OWN_IP.octets(), self.own_port);
        state.enable_foreign_device_registration(ForeignDevicePolicy::default());
        let row = |ip: [u8; 4], port, mask| BdtEntry {
            ip,
            port,
            broadcast_mask: mask,
        };
        state
            .set_bdt(vec![
                row(OWN_IP.octets(), self.own_port, [255; 4]),
                row([198, 51, 100, 1], 0xBAC0, [255; 4]),
                row([203, 0, 113, 1], 0xBAC0, [255, 255, 255, 0]),
            ])
            .unwrap();
        Some(Arc::new(Mutex::new(state)))
    }

    fn limiter(&self) -> Arc<Mutex<ManagementRateLimiter>> {
        let mut limiter = ManagementRateLimiter::new();
        limiter.freeze_clock(std::time::Instant::now());
        Arc::new(Mutex::new(limiter))
    }
}

/// A datagram as the OS reports it.
#[derive(Clone, Debug)]
pub(super) struct Incoming {
    pub(super) payload: Vec<u8>,
    pub(super) source: SocketAddrV4,
    pub(super) destination: IpAddr,
    pub(super) os_group: Option<bool>,
    /// Read by a broadcast listener, with the interface the OS named.
    pub(super) listener: Option<Option<u32>>,
}

impl Incoming {
    fn received(&self) -> (ReceivedDatagram, bool) {
        let received = ReceivedDatagram {
            len: self.payload.len(),
            peer: SocketAddr::V4(self.source),
            destination: self.destination,
            arrival_index: self.listener.flatten(),
            os_group_delivery: self.os_group,
        };
        (received, self.listener.is_some())
    }
}

/// A management request a test makes by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Request {
    ReadBdt,
    WriteBdt,
    ReadFdt,
    DeleteFdtEntry,
    Register(u16),
}

impl Request {
    fn bvll(self) -> Bvll<'static> {
        match self {
            Self::ReadBdt => Bvll::ReadBroadcastDistributionTable,
            Self::WriteBdt => {
                Bvll::WriteBroadcastDistributionTable(BdtEntries::from_wire(&WRITE_ROW).unwrap())
            }
            Self::ReadFdt => Bvll::ReadForeignDeviceTable,
            Self::DeleteFdtEntry => {
                Bvll::DeleteForeignDeviceTableEntry(SocketAddrV4::new(OTHER_LOCAL, 0xBAC0))
            }
            Self::Register(ttl) => Bvll::RegisterForeignDevice { ttl },
        }
    }

    fn expected(self) -> BvlcResponseKind {
        match self {
            Self::ReadBdt => BvlcResponseKind::ReadBroadcastDistributionTableAck,
            Self::ReadFdt => BvlcResponseKind::ReadForeignDeviceTableAck,
            _ => BvlcResponseKind::Result,
        }
    }

    fn ttl(self) -> Option<u16> {
        match self {
            Self::Register(ttl) => Some(ttl),
            _ => None,
        }
    }
}

/// What a helper (`read_bdt` and the others) makes of a response, as text.
pub(super) fn helper_outcome(request: Request, response: Result<BvllMessage, Error>) -> String {
    let decoded = response.and_then(|msg| match request {
        Request::ReadBdt => match msg.message()? {
            Bvll::BvlcResult(code) => Err(Error::Encoding(format!("BVLC-Result: {code:?}"))),
            Bvll::ReadBroadcastDistributionTableAck(table) => {
                bbmd::bdt_from_wire(table).map(|rows| format!("{rows:?}"))
            }
            other => Err(unexpected_response(
                BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
                other.function(),
            )),
        },
        Request::ReadFdt => match msg.message()? {
            Bvll::BvlcResult(code) => Err(Error::Encoding(format!("BVLC-Result: {code:?}"))),
            Bvll::ReadForeignDeviceTableAck(table) => {
                bbmd::fdt_from_wire(table).map(|rows| format!("{rows:?}"))
            }
            other => Err(unexpected_response(
                BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
                other.function(),
            )),
        },
        _ => decode_bvlc_result_code(&msg).map(|code| format!("{code:?}")),
    });
    format!("{decoded:?}")
}

/// How a manual exchange ended: its index among those admitted, when (in
/// milliseconds from the start), and what the helper made of it.
pub(super) type Ended = (usize, u128, String);

/// What one side looks like after a step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Observation {
    pub(super) counters: ReceiveCounters,
    pub(super) group_sources: u64,
    pub(super) group_origins: u64,
    pub(super) client: BvlcClientSnapshot,
    /// NPDUs handed up: the NPDU, its source MAC and `link_layer_group`.
    pub(super) delivered: Vec<(Vec<u8>, Vec<u8>, bool)>,
}

/// One side of the test.
pub(super) trait Side {
    fn sink(&self) -> &SendSink;
    async fn receive(&self, incoming: &Incoming);
    /// Admit a manual exchange; `Err` holds the refusal's text.
    fn begin(&mut self, request: Request, peer: SocketAddrV4) -> Result<(), String>;
    /// Report the oldest admitted exchange sent.
    fn sent(&mut self);
    /// Fail the oldest admitted exchange's send.
    fn fail(&mut self);
    /// Give up the oldest admitted or waiting exchange.
    async fn cancel(&mut self);
    async fn broadcast(&self, npdu: &[u8]) -> Result<(), String>;
    async fn unicast(&self, npdu: &[u8], to: SocketAddrV4) -> Result<(), String>;
    fn observe(&mut self) -> Observation;
    fn ended(&self) -> Vec<Ended>;
}

/// Shared by both sides: the sink, the network layer's channel, the clock
/// the exchanges' ends are timed on, and those ends.
struct Common {
    sink: SendSink,
    socket: Arc<BipSocket>,
    rx: mpsc::Receiver<ReceivedNpdu>,
    start: Instant,
    ended: Arc<Mutex<Vec<Ended>>>,
    admitted: usize,
}

impl Common {
    async fn new() -> (Self, mpsc::Sender<ReceivedNpdu>) {
        let sink = SendSink::default();
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let socket = Arc::new(BipSocket::recording(socket, sink.clone()));
        let (tx, rx) = mpsc::channel(64);
        let common = Self {
            sink,
            socket,
            rx,
            start: Instant::now(),
            ended: Arc::default(),
            admitted: 0,
        };
        (common, tx)
    }

    fn end(&self, index: usize, outcome: String) {
        let at = self.start.elapsed().as_millis();
        self.ended.lock().unwrap().push((index, at, outcome));
    }

    fn delivered(&mut self) -> Vec<(Vec<u8>, Vec<u8>, bool)> {
        let mut delivered = Vec::new();
        while let Ok(npdu) = self.rx.try_recv() {
            let mac = npdu.source_mac.as_slice().to_vec();
            delivered.push((npdu.npdu.to_vec(), mac, npdu.link_layer_group));
        }
        delivered
    }

    /// A task that waits for an exchange's outcome and records it.
    fn wait<F>(&self, index: usize, request: Request, outcome: F) -> JoinHandle<()>
    where
        F: std::future::Future<Output = Result<BvllMessage, Error>> + Send + 'static,
    {
        let (ended, start) = (Arc::clone(&self.ended), self.start);
        tokio::spawn(async move {
            let outcome = helper_outcome(request, outcome.await);
            let at = start.elapsed().as_millis();
            ended.lock().unwrap().push((index, at, outcome));
        })
    }
}

/// A manual exchange in flight.
enum Open<G, R> {
    Admitted {
        index: usize,
        request: Request,
        guard: G,
        response: R,
    },
    Waiting {
        index: usize,
        task: JoinHandle<()>,
    },
}

/// Take the oldest admitted exchange out of `open`.
fn oldest_admitted<G, R>(open: &mut Vec<Open<G, R>>) -> Option<(usize, Request, G, R)> {
    let position = open
        .iter()
        .position(|open| matches!(open, Open::Admitted { .. }))?;
    match open.remove(position) {
        Open::Admitted {
            index,
            request,
            guard,
            response,
        } => Some((index, request, guard, response)),
        Open::Waiting { .. } => unreachable!(),
    }
}

/// Give up the oldest exchange in `open`: drop its guard, or abort the task
/// that holds it and wait until it is gone.
async fn cancel_oldest<G, R>(open: &mut Vec<Open<G, R>>, common: &Common) {
    if open.is_empty() {
        return;
    }
    match open.remove(0) {
        Open::Admitted { index, guard, .. } => {
            drop(guard);
            common.end(index, "cancelled".into());
        }
        Open::Waiting { index, task } => {
            task.abort();
            if task.await.is_err() {
                common.end(index, "cancelled".into());
            }
        }
    }
}

/// The transport as it was.
pub(super) struct Legacy {
    common: Common,
    ctx: old_receive::RecvContext,
    ingress: old_receive::IngressAddresses,
    management: Arc<ManagementClient>,
    foreign: Option<ForeignDeviceConfig>,
    broadcast: SocketAddrV4,
    sources: Arc<AtomicU64>,
    origins: Arc<AtomicU64>,
    open: Vec<Open<RequestGuard, oneshot::Receiver<BvllMessage>>>,
    _worker: Option<JoinHandle<()>>,
}

impl Legacy {
    pub(super) async fn new(config: &Config) -> Self {
        let (common, npdu_tx) = Common::new().await;
        let management = Arc::new(ManagementClient::default());
        management.start();
        let (sources, origins) = (Arc::default(), Arc::default());
        let interface = if config.wildcard {
            Ipv4Addr::UNSPECIFIED
        } else {
            OWN_IP
        };
        let groups = old_receive::BipGroups::new(config.broadcast, config.own_port, interface);
        let ctx = old_receive::RecvContext {
            local_mac: encode_bip_mac(OWN_IP.octets(), config.own_port),
            socket: Arc::clone(&common.socket),
            npdu_tx,
            bbmd: config.bbmd_state(),
            broadcast_addr: config.broadcast,
            broadcast_port: config.own_port,
            client_management: Arc::clone(&management),
            management_limiter: config.limiter(),
            fanout: None,
            group_sources: old_receive::GroupSources::new(
                groups,
                Arc::clone(&sources),
                Arc::clone(&origins),
            ),
            drops: Arc::new(ReceiveDrops::default()),
            force_dbtn_forward_failure: false,
        };
        let ingress = old_receive::IngressAddresses {
            local_ip: OWN_IP,
            unicast_ips: config.unicast_ips(),
            wildcard_bind: config.wildcard,
            listener_interface: config.listener.then_some(LISTENER_INTERFACE),
            interface_mismatch_seen: Default::default(),
        };
        let worker = config.foreign().map(|foreign| {
            let interval = old::interval(foreign).unwrap();
            let run = old::run(
                Arc::clone(&common.socket),
                Arc::clone(&management),
                foreign.clone(),
                interval,
            );
            tokio::spawn(run)
        });
        Self {
            common,
            ctx,
            ingress,
            management,
            foreign: config.foreign().cloned(),
            broadcast: config.broadcast_address(),
            sources,
            origins,
            open: Vec::new(),
            _worker: worker,
        }
    }
}

impl Side for Legacy {
    fn sink(&self) -> &SendSink {
        &self.common.sink
    }

    async fn receive(&self, incoming: &Incoming) {
        let (received, listener) = incoming.received();
        let arrival = if listener {
            old_receive::ArrivalSocket::BroadcastListener
        } else {
            old_receive::ArrivalSocket::Primary
        };
        old_receive::handle_datagram(
            &incoming.payload,
            &received,
            arrival,
            &self.ingress,
            &self.ctx,
        )
        .await;
    }

    fn begin(&mut self, request: Request, peer: SocketAddrV4) -> Result<(), String> {
        let target = (peer.ip().octets(), peer.port());
        let function = request.bvll().function();
        let (guard, response) = self
            .management
            .begin(target, function, request.expected(), request.ttl())
            .map_err(|error| error.to_string())?;
        let index = self.common.admitted;
        self.common.admitted += 1;
        self.open.push(Open::Admitted {
            index,
            request,
            guard,
            response,
        });
        Ok(())
    }

    fn sent(&mut self) {
        let Some((index, request, guard, response)) = oldest_admitted(&mut self.open) else {
            return;
        };
        // `ManagementClient::request` after its send, as of the base.
        let deadline = guard.sent(MANUAL_TIMEOUT);
        let outcome = async move {
            match tokio::time::timeout_at(deadline, response).await {
                Ok(Ok(msg)) => Ok(msg),
                Ok(Err(_)) => Err(Error::Encoding("BVLC response channel dropped".into())),
                Err(_) => {
                    guard.fail(ForeignRegistrationOutcome::TimedOut);
                    Err(Error::Timeout(MANUAL_TIMEOUT))
                }
            }
        };
        let task = self.common.wait(index, request, outcome);
        self.open.push(Open::Waiting { index, task });
    }

    fn fail(&mut self) {
        if let Some((index, _, guard, _)) = oldest_admitted(&mut self.open) {
            guard.fail(ForeignRegistrationOutcome::LocalError);
            self.common.end(index, "failed".into());
        }
    }

    async fn cancel(&mut self) {
        cancel_oldest(&mut self.open, &self.common).await;
    }

    async fn broadcast(&self, npdu: &[u8]) -> Result<(), String> {
        let socket = &self.common.socket;
        old::send_broadcast(socket, self.foreign.as_ref(), self.broadcast, npdu)
            .await
            .map_err(|error| error.to_string())
    }

    async fn unicast(&self, npdu: &[u8], to: SocketAddrV4) -> Result<(), String> {
        let mac = encode_bip_mac(to.ip().octets(), to.port());
        old::send_unicast(&self.common.socket, npdu, &mac)
            .await
            .map_err(|error| error.to_string())
    }

    fn observe(&mut self) -> Observation {
        Observation {
            counters: self.ctx.drops.snapshot(),
            group_sources: self.sources.load(Ordering::Relaxed),
            group_origins: self.origins.load(Ordering::Relaxed),
            client: self.management.snapshot(),
            delivered: self.common.delivered(),
        }
    }

    fn ended(&self) -> Vec<Ended> {
        self.common.ended.lock().unwrap().clone()
    }
}

/// The transport on the node engine.
pub(super) struct Engine {
    common: Common,
    ctx: RecvContext,
    ingress: IngressAddresses,
    open: Vec<Open<Exchange, oneshot::Receiver<Outcome>>>,
    _driver: JoinHandle<()>,
}

impl Engine {
    pub(super) async fn new(config: &Config) -> Self {
        let (common, npdu_tx) = Common::new().await;
        let mode = match &config.mode {
            ModeConfig::Normal => Mode::Normal,
            ModeConfig::Bbmd => Mode::Bbmd,
            ModeConfig::Foreign(foreign) => {
                Mode::Foreign(foreign.engine(foreign.interval().unwrap()))
            }
        };
        let link = Link {
            address: config.own(),
            subnet: LocalSubnet {
                broadcast: config.broadcast_address(),
                mask: None,
            },
            mode,
            budget: None,
        };
        let record = Arc::new(ClientRecord::default());
        record.start();
        let socket = Arc::clone(&common.socket);
        let node = Arc::new(NodeHandle::new(link, socket, record).unwrap());
        let driver = tokio::spawn(Arc::clone(&node).drive());
        let ctx = RecvContext {
            node,
            npdu_tx,
            bbmd: config.bbmd_state(),
            management_limiter: config.limiter(),
            fanout: None,
            group_drops: Default::default(),
            drops: Arc::new(ReceiveDrops::default()),
            force_dbtn_forward_failure: false,
        };
        let ingress = IngressAddresses {
            unicast_ips: config.unicast_ips(),
            wildcard_bind: config.wildcard,
            listener_interface: config.listener.then_some(LISTENER_INTERFACE),
            interface_mismatch_seen: Default::default(),
        };
        Self {
            common,
            ctx,
            ingress,
            open: Vec::new(),
            _driver: driver,
        }
    }
}

impl Side for Engine {
    fn sink(&self) -> &SendSink {
        &self.common.sink
    }

    async fn receive(&self, incoming: &Incoming) {
        let (received, listener) = incoming.received();
        let arrival = if listener {
            Arrival::BroadcastListener
        } else {
            Arrival::Primary
        };
        handle_datagram(
            &incoming.payload,
            &received,
            arrival,
            &self.ingress,
            &self.ctx,
        )
        .await;
    }

    fn begin(&mut self, request: Request, peer: SocketAddrV4) -> Result<(), String> {
        let (guard, response) = self
            .ctx
            .node
            .begin(peer, request.bvll(), MANUAL_TIMEOUT)
            .map_err(|error| error.to_string())?;
        let index = self.common.admitted;
        self.common.admitted += 1;
        self.open.push(Open::Admitted {
            index,
            request,
            guard,
            response,
        });
        Ok(())
    }

    fn sent(&mut self) {
        let Some((index, request, guard, response)) = oldest_admitted(&mut self.open) else {
            return;
        };
        // `NodeHandle::request` after its send.
        guard.sent();
        let outcome = async move {
            let outcome = match response.await {
                Ok(outcome) => outcome,
                Err(_) => Err(Error::Encoding("BVLC response channel dropped".into())),
            };
            drop(guard);
            outcome
        };
        let task = self.common.wait(index, request, outcome);
        self.open.push(Open::Waiting { index, task });
    }

    fn fail(&mut self) {
        if let Some((index, _, guard, _)) = oldest_admitted(&mut self.open) {
            guard.failed();
            self.common.end(index, "failed".into());
        }
    }

    async fn cancel(&mut self) {
        cancel_oldest(&mut self.open, &self.common).await;
    }

    async fn broadcast(&self, npdu: &[u8]) -> Result<(), String> {
        let node = &self.ctx.node;
        node.send_broadcast(npdu, None)
            .await
            .map_err(|error| error.to_string())
    }

    async fn unicast(&self, npdu: &[u8], to: SocketAddrV4) -> Result<(), String> {
        let mac = encode_bip_mac(to.ip().octets(), to.port());
        let node = &self.ctx.node;
        node.send_unicast(npdu, &mac)
            .await
            .map_err(|error| error.to_string())
    }

    fn observe(&mut self) -> Observation {
        Observation {
            counters: self.ctx.drops.snapshot(),
            group_sources: self.ctx.group_drops.sources.load(Ordering::Relaxed),
            group_origins: self.ctx.group_drops.origins.load(Ordering::Relaxed),
            client: self.ctx.node.snapshot(),
            delivered: self.common.delivered(),
        }
    }

    fn ended(&self) -> Vec<Ended> {
        self.common.ended.lock().unwrap().clone()
    }
}
