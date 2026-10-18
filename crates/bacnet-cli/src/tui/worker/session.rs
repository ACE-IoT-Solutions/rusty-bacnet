//! Build the client for the chosen transport, then serve UI commands and
//! forward client notifications until the UI closes the command channel.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_services::who_is::WhoIsRequest;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::UnconfirmedServiceChoice;
use bacnet_types::error::Error;
use bytes::BytesMut;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::browse::limits::RateCap;
use super::watch::WatchOptions;
use super::{DeviceFeed, EventSink, COMMAND_CHANNEL_CAPACITY};
use crate::core::target::AddressStyle;
use crate::transport::{self, TransportArgs};
use crate::tui::message::{
    Command, DirectedTo, OpId, OpOutcome, WhoIsScope, WhoIsSpec, WorkerEvent,
};
use crate::tui::point::BrowseResult;

/// How often the worker checks whether the UI needs a resync.
const RESYNC_EVERY: Duration = Duration::from_secs(1);

/// Which client to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportKind {
    /// BACnet/IP.
    Bip,
    /// BACnet/IPv6.
    Bip6,
    /// BACnet/SC.
    Sc,
}

impl TransportKind {
    /// Status bar label.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Bip => "BIP",
            Self::Bip6 => "BIP6",
            Self::Sc => "SC",
        }
    }

    /// How this transport's MACs are shown.
    pub(crate) fn style(self) -> AddressStyle {
        match self {
            Self::Bip => AddressStyle::Bip,
            Self::Bip6 => AddressStyle::Bip6,
            Self::Sc => AddressStyle::Hex,
        }
    }
}

/// What the worker needs to connect.
#[derive(Clone)]
pub(crate) struct ConnectPlan {
    /// The transport.
    pub(crate) kind: TransportKind,
    /// The global flags.
    pub(crate) args: TransportArgs,
    /// Wait for a [`Command::Connect`] from the interface picker first.
    pub(crate) await_interface: bool,
}

/// A session's request limits and watch list settings.
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionOptions {
    /// Confirmed requests a second across the browser and the watch list
    /// (0: no cap).
    pub(crate) max_rate: u32,
    /// The watch list's settings.
    pub(crate) watch: WatchOptions,
}

/// Capacity of the session's queue to the watch worker: a burst of adds
/// and removes must not be refused, since a lost remove would leave a
/// subscription until the session ends.
const WATCH_CHANNEL_CAPACITY: usize = 128;

/// Start the worker task. Boxed so the transport futures never sit on a
/// small stack.
pub(crate) fn spawn(
    plan: ConnectPlan,
    sink: EventSink,
    commands: mpsc::Receiver<Command>,
    options: SessionOptions,
) -> JoinHandle<()> {
    tokio::spawn(Box::pin(connect(plan, sink, commands, options)))
}

async fn connect(
    plan: ConnectPlan,
    sink: EventSink,
    commands: mpsc::Receiver<Command>,
    mut options: SessionOptions,
) {
    let args = plan.args;
    // The CLI's clients keep bacnet-client's default of three retries.
    options.watch.request_budget = Duration::from_millis(args.timeout_ms) * 4;
    let style = plan.kind.style();
    match plan.kind {
        TransportKind::Bip => {
            if let Some(mut client) =
                finish_build(Box::pin(transport::build_bip_client(&args)), &sink).await
            {
                if let Some(foreign) = &args.foreign_device {
                    let bbmd = std::net::SocketAddrV4::new(foreign.bbmd_ip, foreign.bbmd_port);
                    let outcome = tokio::select! {
                        biased;
                        () = sink.cancelled() => None,
                        result = crate::foreign::await_registration(&client, bbmd) => Some(result),
                    };
                    let Some(outcome) = outcome else {
                        if let Err(error) = client.stop().await {
                            tracing::warn!("client stop: {error}");
                        }
                        return;
                    };
                    match outcome {
                        Ok(()) => tracing::info!("BBMD {bbmd} accepted the foreign-device registration; it renews automatically"),
                        Err(error) => tracing::warn!("{error}; retrying in the background"),
                    }
                }
                Box::pin(serve(client, style, sink, commands, options)).await;
            }
        }
        TransportKind::Bip6 => {
            if let Some(client) =
                finish_build(Box::pin(transport::build_bip6_client(&args)), &sink).await
            {
                Box::pin(serve(client, style, sink, commands, options)).await;
            }
        }
        #[cfg(feature = "sc-tls")]
        TransportKind::Sc => {
            if let Some(client) =
                finish_build(Box::pin(transport::build_sc_client(&args)), &sink).await
            {
                Box::pin(serve(client, style, sink, commands, options)).await;
            }
        }
        #[cfg(not(feature = "sc-tls"))]
        TransportKind::Sc => {
            failed(
                &sink,
                Error::Encoding("BACnet/SC requires the 'sc-tls' feature".into()),
            )
            .await;
        }
    }
}

/// Retain a partially completed builder through cancellation: it may already
/// own resources. Finish construction, then stop fully without publishing it.
pub(crate) async fn finish_build<T: TransportPort + 'static>(
    builder: Pin<Box<impl Future<Output = Result<BACnetClient<T>, Error>>>>,
    sink: &EventSink,
) -> Option<BACnetClient<T>> {
    match builder.await {
        Ok(mut client) if sink.is_cancelled() => {
            if let Err(error) = client.stop().await {
                tracing::warn!("client stop: {error}");
            }
            None
        }
        Ok(client) => Some(client),
        Err(error) => {
            failed(sink, error).await;
            None
        }
    }
}

async fn failed(sink: &EventSink, error: Error) {
    tracing::error!("connect failed: {error}");
    sink.deliver(WorkerEvent::ConnectFailed {
        error: error.to_string(),
    })
    .await;
}

/// A directed Who-Is waiting for the VMAC of the B/IPv6 node it goes to.
///
/// The worker loop polls the lookup beside its other branches, so Cancel,
/// device events and a newer Who-Is don't wait the up to three seconds a
/// Virtual-Address-Resolution can take (U.5). The lookup holds no lock while
/// it waits; dropping it gives up the wait.
struct Resolving<'a> {
    op: OpId,
    spec: WhoIsSpec,
    vmac: Pin<Box<dyn Future<Output = Result<Vec<u8>, String>> + Send + 'a>>,
}

/// The VMAC `resolving` waits for. Never ready while there is none, so the
/// loop's branch for it stays idle.
async fn resolved(resolving: &mut Option<Resolving<'_>>) -> Result<Vec<u8>, String> {
    match resolving {
        Some(lookup) => lookup.vmac.as_mut().await,
        None => std::future::pending().await,
    }
}

/// Serve one connected client until the UI drops its command sender.
///
/// The client is shared with the watch list's renewal tasks while the
/// session runs. The watch worker stops every one of them before it
/// returns, so the client is this function's alone again when it stops it.
pub(crate) async fn serve<T: TransportPort + 'static>(
    client: BACnetClient<T>,
    style: AddressStyle,
    sink: EventSink,
    mut commands: mpsc::Receiver<Command>,
    options: SessionOptions,
) {
    let client = Arc::new(client);
    let rate = RateCap::new(options.max_rate);
    let mut local = style.format(client.local_mac());
    // A B/IPv6 VMAC alone doesn't say where the client is bound.
    if let Some(bound) =
        crate::core::target::as_bip6(client.transport()).and_then(|t| t.local_address())
    {
        local = format!("{local} ({bound})");
    }
    tracing::info!("connected; local address {local}");
    let mut devices = client.device_events();
    let mut collisions = client.device_collision_events();
    if sink.deliver(WorkerEvent::Connected { local }).await {
        let (browse, browse_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let (watch, watch_rx) = mpsc::channel(WATCH_CHANNEL_CAPACITY);
        // The browser and the watch list run beside this loop under
        // `join!`, so their jobs keep being polled while an arm here awaits;
        // the loop ending drops `browse` and `watch`, which ends them.
        let session = async {
            let browse = browse;
            let watch = watch;
            let mut feed = DeviceFeed::new(style);
            let mut listening: Option<(OpId, Instant)> = None;
            let mut resolving: Option<Resolving<'_>> = None;
            let mut resync = tokio::time::interval(RESYNC_EVERY);
            resync.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let deadline = listening.map_or_else(far_future, |(_, until)| until);
                tokio::select! {
                    biased;
                    () = sink.cancelled() => break,
                    command = commands.recv() => match command {
                        Some(Command::WhoIs { op, spec }) => {
                            let superseded = listening.take().map(|(old, _)| old);
                            let superseded = superseded.or(resolving.take().map(|lookup| lookup.op));
                            if let Some(old) = superseded {
                                sink.deliver(WorkerEvent::OpFinished { op: old, outcome: OpOutcome::Cancelled }).await;
                            }
                            if let WhoIsScope::Directed { to: DirectedTo::Bip6(peer), .. } = &spec.scope {
                                let vmac = Box::pin(crate::core::target::bip6_vmac(client.transport(), *peer));
                                resolving = Some(Resolving { op, spec, vmac });
                            } else {
                                listening = who_is(&client, &sink, op, &spec).await;
                            }
                        }
                        Some(Command::Cancel { op }) => {
                            let running = listening.is_some_and(|(id, _)| id == op)
                                || resolving.as_ref().is_some_and(|lookup| lookup.op == op);
                            if running {
                                listening = None;
                                resolving = None;
                                tracing::info!("Who-Is {op} cancelled");
                                sink.deliver(WorkerEvent::OpFinished { op, outcome: OpOutcome::Cancelled }).await;
                            }
                        }
                        Some(command @ (Command::Browse { .. } | Command::StopBrowse { .. })) => {
                            if let Err(refused) = browse.try_send(command) {
                                let refused = refused.into_inner();
                                if let Command::StopBrowse { device } = refused {
                                    tracing::debug!("browser busy; dropped the stop for device {device}");
                                }
                                if let Command::Browse { req, .. } = refused {
                                    let result = BrowseResult::Failed("the browser is busy; try again".into());
                                    sink.deliver(WorkerEvent::Browse { req, result }).await;
                                }
                            }
                        }
                        Some(command @ Command::Watch(_)) => {
                            // The watch worker is polled beside this loop, so
                            // waiting for room can't deadlock; it only fails
                            // once the worker has ended with the session.
                            if watch.send(command).await.is_err() {
                                tracing::debug!("the watch list has ended");
                            }
                        }
                        Some(Command::Disconnect) => break,
                        Some(Command::Connect { .. } | Command::Reconnect | Command::Profile(_)) => {}
                        None => break,
                    },
                    vmac = resolved(&mut resolving), if resolving.is_some() => {
                        if let Some(Resolving { op, spec, .. }) = resolving.take() {
                            listening = match vmac {
                                Ok(mac) => who_is(&client, &sink, op, &spec.with_vmac(mac)).await,
                                Err(error) => {
                                    failed_who_is(&sink, op, error).await;
                                    None
                                }
                            };
                        }
                    }
                    received = devices.recv(), if feed.devices_open => feed.on_device(&sink, received),
                    received = collisions.recv(), if feed.collisions_open => {
                        if let Some(event) = feed.on_collision(&sink, received) {
                            sink.deliver(event).await;
                        }
                    }
                    () = tokio::time::sleep_until(deadline), if listening.is_some() => {
                        if let Some((op, _)) = listening.take() {
                            sink.deliver(WorkerEvent::OpFinished { op, outcome: OpOutcome::Completed }).await;
                        }
                    }
                    _ = resync.tick(), if feed.needs_resync => {
                        let rows = client.discovered_devices().await.iter().map(|d| feed.row(d)).collect();
                        if sink.offer(WorkerEvent::Snapshot(rows)) == super::Offer::Sent {
                            feed.needs_resync = false;
                        }
                    }
                }
            }
        };
        tokio::join!(
            Box::pin(session),
            Box::pin(super::browse::run(&client, &sink, browse_rx, &rate)),
            Box::pin(super::watch::run(
                &client,
                &sink,
                watch_rx,
                &rate,
                &options.watch
            ))
        );
    }
    stop_shared(client).await;
}

/// How long [`stop_shared`] waits quietly for the renewal tasks to let go
/// of the client, and then how long more after warning.
const RELEASE_WAIT: (Duration, Duration) = (Duration::from_secs(1), Duration::from_secs(10));

/// Stop the client once no renewal task holds it.
///
/// The watch worker has told every renewal task to stop before this runs,
/// and a stopping task lets go of the client as it ends. Dropping the client
/// instead of stopping it would leave the transport's sockets to whatever
/// task still held them, so a reconnect could find the port taken: this
/// waits for the tasks, warning after a second. Only a task that never
/// ends would make it give up, with an error, after the longer wait.
async fn stop_shared<T: TransportPort + 'static>(mut client: Arc<BACnetClient<T>>) {
    let start = Instant::now();
    let mut warned = false;
    loop {
        match Arc::try_unwrap(client) {
            Ok(mut owned) => {
                if let Err(error) = owned.stop().await {
                    tracing::warn!("client stop: {error}");
                }
                return;
            }
            Err(shared) => client = shared,
        }
        let waited = start.elapsed();
        if waited >= RELEASE_WAIT.0 + RELEASE_WAIT.1 {
            tracing::error!(
                "a COV renewal task still holds the client after {} s; dropping it unstopped",
                waited.as_secs()
            );
            return;
        }
        if waited >= RELEASE_WAIT.0 && !warned {
            warned = true;
            tracing::warn!("waiting for a COV renewal task to release the client");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn far_future() -> Instant {
    Instant::now() + Duration::from_secs(86_400)
}

/// Send a Who-Is; on success the listen window to wait out.
async fn who_is<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    sink: &EventSink,
    op: OpId,
    spec: &WhoIsSpec,
) -> Option<(OpId, Instant)> {
    let result = tokio::select! {
        biased;
        () = sink.cancelled() => return None,
        result = send_who_is(client, spec) => result,
    };
    match result {
        Ok(()) => {
            tracing::info!("Who-Is {op} sent ({})", spec.describe());
            sink.deliver(WorkerEvent::WhoIsSent { op }).await;
            Some((op, Instant::now() + spec.listen))
        }
        Err(error) => {
            failed_who_is(sink, op, error).await;
            None
        }
    }
}

async fn failed_who_is(sink: &EventSink, op: OpId, error: String) {
    tracing::warn!("Who-Is {op} failed: {error}");
    sink.deliver(WorkerEvent::OpFinished {
        op,
        outcome: OpOutcome::Failed(error),
    })
    .await;
}

/// Send the Who-Is `spec` describes. A directed one to a B/IPv6 address
/// must have had its VMAC found first ([`WhoIsSpec::with_vmac`]).
async fn send_who_is<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    spec: &WhoIsSpec,
) -> Result<(), String> {
    let range = spec.range;
    let sent = match &spec.scope {
        WhoIsScope::Local => {
            let mut buf = BytesMut::new();
            WhoIsRequest { range }.encode(&mut buf);
            client
                .broadcast_unconfirmed(UnconfirmedServiceChoice::WHO_IS, &buf)
                .await
        }
        WhoIsScope::Global => client.who_is(range).await,
        WhoIsScope::Directed {
            to: DirectedTo::Mac(mac),
            ..
        } => client.who_is_directed(mac, range).await,
        WhoIsScope::Directed {
            to: DirectedTo::Bip6(peer),
            ..
        } => return Err(format!("the VMAC at {peer} has not been found")),
        WhoIsScope::Network(dnet) => client.who_is_network(*dnet, range).await,
    };
    sent.map_err(|error| error.to_string())
}
