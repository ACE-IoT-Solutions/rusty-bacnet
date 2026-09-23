//! Shared BACnet/SC transport state transitions and publication helpers.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;

use super::{ScConnection, ScConnectionState, ScTransport, WebSocketPort, DEFAULT_MAX_APDU_LENGTH};
use crate::port::TransportHealthState;

const BACNET_NPDU_BASE_HEADER_LEN: u16 = 2;
const SC_ENCAPSULATED_NPDU_BASE_HEADER_LEN: u16 = 10;

fn normalize_hub_url(value: &str) -> String {
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn normalize_authority(value: &str, omit_port: u16) -> String {
        let (host, port) = if let Some(rest) = value.strip_prefix('[') {
            match rest.split_once(']') {
                Some((host, suffix)) => {
                    let port = suffix.strip_prefix(':').and_then(|port| port.parse().ok());
                    (format!("[{host}]"), port)
                }
                None => (value.to_owned(), None),
            }
        } else if let Some((host, port)) = value.rsplit_once(':') {
            match port.parse::<u16>() {
                Ok(port) => (host.to_owned(), Some(port)),
                Err(_) => (value.to_owned(), None),
            }
        } else {
            (value.to_owned(), None)
        };
        let host = if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            inner
                .parse::<Ipv6Addr>()
                .map(|address| format!("[{address}]"))
                .unwrap_or_else(|_| host.to_ascii_lowercase())
        } else {
            host.parse::<Ipv4Addr>()
                .map(|address| address.to_string())
                .unwrap_or_else(|_| host.to_ascii_lowercase())
        };
        match port {
            Some(port) if port != omit_port => format!("{host}:{port}"),
            _ => host,
        }
    }

    let value = value.trim();
    let value = value.split_once('#').map_or(value, |(base, _)| base);
    let Some((scheme, remainder)) = value.split_once("://") else {
        return value.to_owned();
    };
    let authority_end = remainder.find(['/', '?']).unwrap_or(remainder.len());
    let (authority, suffix) = remainder.split_at(authority_end);
    let scheme = scheme.to_ascii_lowercase();
    let authority = normalize_authority(authority, u16::from(scheme == "wss") * 443);
    let tail_start = suffix.find('?').unwrap_or(suffix.len());
    let (path, tail) = suffix.split_at(tail_start);
    let path = path.trim_end_matches('/');
    let path = if path.is_empty() && !tail.is_empty() {
        "/"
    } else {
        path
    };
    format!("{scheme}://{authority}{path}{tail}")
}

impl<W: WebSocketPort> ScTransport<W> {
    /// Attach stable hub URLs used to identify this SC topology.
    pub fn with_hub_urls(
        mut self,
        primary_hub_url: impl AsRef<str>,
        failover_hub_url: Option<impl AsRef<str>>,
    ) -> Self {
        let primary = normalize_hub_url(primary_hub_url.as_ref());
        let failover = failover_hub_url.map(|url| normalize_hub_url(url.as_ref()));
        self.topology_collision_ids = match &failover {
            Some(url) => vec![primary.clone(), url.clone()],
            None => vec![primary.clone()],
        };
        self.topology_id = Some(match failover {
            Some(url) => format!("{primary}|{url}"),
            None => primary,
        });
        self
    }

    /// Get the connection state (for testing/inspection).
    ///
    /// This exposes mutable connection fields, including identity. Startup
    /// validation does not protect against later application mutation here.
    pub fn connection(&self) -> Option<&Arc<Mutex<ScConnection>>> {
        self.connection.as_ref()
    }

    /// Subscribe to BACnet/SC connection state changes.
    ///
    /// The returned watch receiver yields the latest known state immediately and
    /// change notifications for subsequent state updates observed by the
    /// transport. Rapid updates can coalesce under Tokio watch semantics, so use
    /// this as a current-state signal rather than a durable transition log.
    pub fn connection_state_changes(&self) -> watch::Receiver<ScConnectionState> {
        self.state_tx.subscribe()
    }

    /// Subscribe to the current operational SC health snapshot.
    pub fn transport_health_changes(&self) -> watch::Receiver<crate::port::TransportHealth> {
        self.health_tx.subscribe()
    }
}

pub(super) fn abort_background_task_and_drop_sockets<W: WebSocketPort>(
    transport: &mut ScTransport<W>,
) -> (Option<JoinHandle<()>>, Option<JoinHandle<()>>) {
    let task = transport.recv_task.take();
    if let Some(task) = &task {
        task.abort();
    }
    let restore_task = transport
        .restore_disconnect_task
        .lock()
        .ok()
        .and_then(|mut task| task.take());
    if let Some(task) = &restore_task {
        task.abort();
    }
    if let Some(conn) = &transport.connection {
        if let Ok(mut c) = conn.try_lock() {
            c.state = ScConnectionState::Disconnected;
        }
    }
    transport
        .effective_max_apdu_length
        .store(DEFAULT_MAX_APDU_LENGTH, Ordering::Relaxed);
    transport
        .state_tx
        .send_replace(ScConnectionState::Disconnected);
    transport.health_tx.send_modify(|health| {
        health.state = TransportHealthState::Down;
        health.active_hub = None;
        health.since = None;
    });
    transport.ws_shared = None;
    transport.connection = None;
    transport.ws = None;
    transport.failover_ws = None;
    (task, restore_task)
}

fn effective_max_apdu_length(conn: &ScConnection) -> u16 {
    let bvlc_npdu_budget = conn
        .hub_max_bvlc_length
        .saturating_sub(SC_ENCAPSULATED_NPDU_BASE_HEADER_LEN);
    let effective_npdu = conn.hub_max_apdu_length.min(bvlc_npdu_budget);
    effective_npdu.saturating_sub(BACNET_NPDU_BASE_HEADER_LEN)
}

pub(crate) fn publish_effective_max_apdu_length(store: &AtomicU16, conn: &ScConnection) {
    store.store(effective_max_apdu_length(conn), Ordering::Relaxed);
}

pub(crate) async fn connect_probe_from(
    conn: &Arc<Mutex<ScConnection>>,
) -> Arc<Mutex<ScConnection>> {
    Arc::new(Mutex::new(conn.lock().await.connect_probe()))
}

pub(crate) async fn absorb_failed_connect_probe(
    conn: &Arc<Mutex<ScConnection>>,
    probe_conn: &Arc<Mutex<ScConnection>>,
) {
    let probe = probe_conn.lock().await;
    let mut c = conn.lock().await;
    c.absorb_failed_probe(&probe);
}

pub(crate) async fn publish_connected_ws<W: WebSocketPort>(
    conn: &Arc<Mutex<ScConnection>>,
    active_ws: &Arc<Mutex<Arc<W>>>,
    ws: &Arc<W>,
    probe_conn: &Arc<Mutex<ScConnection>>,
    state_tx: &watch::Sender<ScConnectionState>,
    effective_max_apdu_length: &AtomicU16,
) {
    let restored = probe_conn.lock().await.clone();
    let mut current = active_ws.lock().await;
    let mut c = conn.lock().await;
    *c = restored;
    *current = ws.clone();
    publish_effective_max_apdu_length(effective_max_apdu_length, &c);
    state_tx.send_replace(c.state);
}
