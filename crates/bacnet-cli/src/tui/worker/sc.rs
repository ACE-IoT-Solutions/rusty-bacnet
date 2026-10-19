//! The SC screen's worker (#967): follows the hub connector through the
//! transport handle (#956) and the observability API (#959).
//!
//! It runs beside the session loop under `join!` and ends when the loop
//! drops its stop sender. Each event becomes a report and a log line. The
//! link status is polled once a second. The transport's broadcast never
//! waits for this worker: when the worker falls behind and the stream
//! reports `Lagged`, it reports a gap and resyncs from `connection_status()`.
//! The hub's certificates are read after each connect, under a timeout, so
//! a busy connection lock cannot hold the worker; nothing else is locked or
//! awaited meanwhile.

use std::future::Future;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_transport::port::TransportPort;
use bacnet_transport::sc::ScEvent;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use super::EventSink;
use crate::tui::app::sc::{describe, Severity};
use crate::tui::message::WorkerEvent;
use crate::tui::sc::{CertSummary, LinkEvent, LinkSnapshot, ScReport};

/// How often the link status is polled; heartbeat ages show seconds.
const STATUS_EVERY: Duration = Duration::from_secs(1);

/// Follow the hub connector until `stop` closes, when the client is SC.
pub(crate) async fn run<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    sink: &EventSink,
    stop: mpsc::Receiver<()>,
) {
    #[cfg(feature = "sc-tls")]
    {
        use std::any::Any;
        type Sc = bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>;
        if let Some(sc) = (client.transport() as &dyn Any).downcast_ref::<Sc>() {
            /// The longest the worker waits to read the hub's certificates.
            const CHAIN_WAIT: Duration = Duration::from_secs(2);
            // Subscribe before the first snapshot, so nothing falls between.
            let events = sc.connection_events();
            let status = || sc.connection_status().into();
            let chain = || async {
                // A timeout drops the lock wait with the future.
                let info = tokio::time::timeout(CHAIN_WAIT, sc.hub_peer_info()).await;
                info.ok()
                    .flatten()
                    .map(|info| crate::tui::sc::summarize(info.tls().certificates()))
            };
            pump(events, status, chain, sink, stop).await;
        }
    }
    #[cfg(not(feature = "sc-tls"))]
    let _ = (client, sink, stop);
}

fn log(event: &LinkEvent) {
    let (severity, text) = describe(event);
    match severity {
        Severity::Info => tracing::info!("SC: {text}"),
        Severity::Warn => tracing::warn!("SC: {text}"),
        Severity::Error => tracing::error!("SC: {text}"),
    }
}

/// The worker's loop over `events`, with `status` for snapshots and
/// `chain` for the hub's certificates.
pub(crate) async fn pump<S, C, F>(
    mut events: broadcast::Receiver<ScEvent>,
    status: S,
    chain: C,
    sink: &EventSink,
    mut stop: mpsc::Receiver<()>,
) where
    S: Fn() -> LinkSnapshot,
    C: Fn() -> F,
    F: Future<Output = Option<Vec<CertSummary>>>,
{
    let snapshot = || {
        sink.offer(WorkerEvent::Sc(ScReport::Status(status())));
    };
    let certificates = || async {
        if let Some(chain) = chain().await {
            sink.deliver(WorkerEvent::Sc(ScReport::Chain(chain))).await;
        }
    };
    snapshot();
    certificates().await;
    let mut poll = tokio::time::interval(STATUS_EVERY);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut open = true;
    loop {
        tokio::select! {
            biased;
            () = sink.cancelled() => break,
            _ = stop.recv() => break,
            received = events.recv(), if open => match received {
                Ok(event) => {
                    let Some(event) = LinkEvent::from_event(event) else { continue };
                    log(&event);
                    let connected = matches!(event, LinkEvent::Connected { .. });
                    sink.deliver(WorkerEvent::Sc(ScReport::Event(event))).await;
                    if connected {
                        certificates().await;
                    }
                }
                Err(RecvError::Lagged(missed)) => {
                    tracing::warn!("SC: {missed} hub-connector events missed; resyncing");
                    sink.add_dropped(missed);
                    sink.deliver(WorkerEvent::Sc(ScReport::Gap { missed })).await;
                    snapshot();
                    certificates().await;
                }
                Err(RecvError::Closed) => open = false,
            },
            _ = poll.tick() => snapshot(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_transport::sc::ScConnectionState;

    fn snapshot() -> LinkSnapshot {
        LinkSnapshot {
            state: ScConnectionState::Connecting,
            hub: Some(bacnet_transport::sc::ScActiveHub::Failover),
            hub_vmac: None,
            heartbeat_age: None,
            heartbeat_rtt: None,
        }
    }

    /// A worker that fell behind reports the gap, then resyncs the status,
    /// then goes on with the events still held; the sender never waited.
    #[tokio::test(start_paused = true)]
    async fn a_lagged_stream_reports_a_gap_and_resyncs() {
        let (tx, events) = broadcast::channel(2);
        for _ in 0..5 {
            tx.send(ScEvent::FailoverEngaged).unwrap();
        }
        let (sink, mut ui) = EventSink::channel(64);
        let (stop_tx, stop) = mpsc::channel(1);
        let pumped = pump(events, snapshot, || async { None }, &sink, stop);
        let reports = async {
            let mut seen = Vec::new();
            while seen.len() < 5 {
                match ui.recv().await {
                    Some(WorkerEvent::Sc(report)) => seen.push(report),
                    other => panic!("unexpected {other:?}"),
                }
            }
            drop(stop_tx);
            seen
        };
        let ((), seen) = tokio::join!(pumped, reports);
        assert_eq!(
            seen,
            [
                ScReport::Status(snapshot()),
                ScReport::Gap { missed: 3 },
                ScReport::Status(snapshot()),
                ScReport::Event(LinkEvent::FailoverEngaged),
                ScReport::Event(LinkEvent::FailoverEngaged),
            ]
        );
        assert_eq!(
            sink.dropped_counter()
                .load(std::sync::atomic::Ordering::Relaxed),
            3
        );
    }
}
