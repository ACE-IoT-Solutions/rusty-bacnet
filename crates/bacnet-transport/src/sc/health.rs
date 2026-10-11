//! BACnet/SC transport-health state transitions.

use std::fmt::Display;
use std::time::Instant;

use tokio::sync::watch;

use crate::port::{TransportHealth, TransportHealthState};

pub(super) fn connecting(tx: &watch::Sender<TransportHealth>) {
    tx.send_replace(TransportHealth {
        state: TransportHealthState::Connecting,
        detail: Some("connecting to hub".into()),
        ..TransportHealth::default()
    });
}

pub(super) fn failed(tx: &watch::Sender<TransportHealth>, detail: &str, error: impl Display) {
    tx.send_replace(TransportHealth {
        state: TransportHealthState::Failed,
        detail: Some(detail.into()),
        last_error: Some(error.to_string()),
        ..TransportHealth::default()
    });
}

pub(super) fn up(tx: &watch::Sender<TransportHealth>, active_hub: &str) {
    tx.send_replace(TransportHealth {
        state: TransportHealthState::Up,
        detail: Some("connected".into()),
        active_hub: Some(active_hub.into()),
        last_error: None,
        attempt: 0,
        since: Some(Instant::now()),
    });
}

pub(super) fn connection_lost(
    tx: &watch::Sender<TransportHealth>,
    will_reconnect: bool,
    reason: String,
) {
    tx.send_modify(|health| {
        health.state = if will_reconnect {
            TransportHealthState::Reconnecting
        } else {
            TransportHealthState::Failed
        };
        health.detail = Some("connection lost".into());
        health.last_error = Some(reason);
        health.since = None;
    });
}

pub(super) fn reconnect_terminated(tx: &watch::Sender<TransportHealth>) {
    tx.send_modify(|health| {
        health.state = TransportHealthState::Failed;
        health.detail = Some("reconnection terminated".into());
        health.active_hub = None;
        health.since = None;
        health
            .last_error
            .get_or_insert_with(|| "reconnect attempts exhausted".into());
    });
}
