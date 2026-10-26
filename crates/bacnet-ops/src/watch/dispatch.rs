//! The hub's one receiver of COV notifications: each goes to the entry
//! whose subscriber process identifier it carries, if it comes from that
//! entry's device and names its object (and property). The loop's arms
//! never await: applying a notification takes two std mutexes in turn (the
//! table's, released before the entry's), and a held-back signal is a
//! deadline the loop sleeps toward.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bacnet_client::client::{CovTarget, ReceivedCOVNotification};
use bacnet_types::MacAddr;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::watch;
use tokio::time::Instant;

use super::{Point, Registry, Shared, Watched};
use crate::format::RawRow;

/// Route notifications until the hub stops or the client closes its
/// channel.
pub(super) async fn run(
    registry: Arc<Registry>,
    mut notifications: broadcast::Receiver<ReceivedCOVNotification>,
    mut stop: watch::Receiver<bool>,
) {
    let mut held: Vec<(Instant, Arc<Shared>)> = Vec::new();
    loop {
        let due = held.iter().map(|(at, _)| *at).min();
        tokio::select! {
            biased;
            _ = stop.wait_for(|stopped| *stopped) => break,
            received = notifications.recv() => match received {
                Ok(received) => held.extend(registry.deliver(&received)),
                Err(RecvError::Lagged(missed)) => {
                    registry.lagged.fetch_add(missed, Ordering::Relaxed);
                }
                Err(RecvError::Closed) => break,
            },
            () = tokio::time::sleep_until(due.unwrap_or_else(Instant::now)), if due.is_some() => {
                let now = Instant::now();
                held.retain(|(at, shared)| {
                    if *at <= now {
                        shared.flush();
                        false
                    } else {
                        true
                    }
                });
            }
        }
    }
}

/// True when `received` is for `point`: from its device's address (see
/// [`CovTarget`]), about its object, and for a property watch, carrying
/// that property.
pub(super) fn belongs(
    point: &Point,
    target: Option<&CovTarget>,
    received: &ReceivedCOVNotification,
) -> bool {
    let notification = &received.notification;
    if notification.monitored_object_identifier != point.object {
        return false;
    }
    if let Watched::Property { property, .. } = point.watched
        && !notification
            .list_of_values
            .iter()
            .any(|value| value.property_identifier == property)
    {
        return false;
    }
    let local_from = |mac: &MacAddr| {
        received.source_network.is_none()
            && received.source_address.is_none()
            && received.source_mac == *mac
    };
    match target {
        Some(CovTarget::Local { mac }) => local_from(mac),
        // A routed device on the client's own network is sent to as a
        // local one, and may answer from its address as the MAC.
        Some(CovTarget::Routed {
            network, address, ..
        }) => {
            (received.source_network == Some(*network)
                && received.source_address.as_ref() == Some(address))
                || local_from(address)
        }
        _ => false,
    }
}

/// Fold a notification's values into `rows`: each reported property
/// replaces the row of the same property, or joins the rows. A property
/// watch keeps only its property.
pub(super) fn merge(rows: &mut Vec<RawRow>, point: &Point, received: &ReceivedCOVNotification) {
    for value in &received.notification.list_of_values {
        let property = value.property_identifier;
        if let Watched::Property {
            property: watched, ..
        } = point.watched
            && watched != property
        {
            continue;
        }
        let row = (property, Ok(value.value.clone()));
        match rows.iter_mut().find(|(p, _)| *p == property) {
            Some(slot) => *slot = row,
            None => rows.push(row),
        }
    }
}
