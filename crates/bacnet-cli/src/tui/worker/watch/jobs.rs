//! The watch list's requests to one device: starting a row (a first read,
//! then a managed COV subscription), cancelling one, and polling a batch.
//!
//! Each runs as one job, and a device runs one job at a time. Every request
//! also takes its turn at the device's gate in the shared
//! [`RequestLimits`], with a slot under the rate cap, so the watch list, the
//! browser and the writes never have more than one confirmed request
//! outstanding to a device between them (#1791). The managed
//! subscription's renewals, sent by its task once per lifetime, take the
//! same gate, though no rate slot. A routed device is subscribed to like a
//! local one, through its router (#1790).

use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bacnet_client::client::{
    BACnetClient, CovPropertySubscription, CovTarget, ManagedCOVSubscription,
    ManagedCOVSubscriptionOptions,
};
use bacnet_ops::engine::{Halt, Reader, unreachable};
use bacnet_ops::limits::RequestLimits;
use bacnet_ops::watch::first::first_read;
use bacnet_ops::watch::poll::{self, PollRow};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;

use super::sample::{self, Reading, RowFormat};
use crate::tui::watched::{Point, RowId};

/// A subscription's lifetime and how long before its end it is renewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lifetime {
    /// Seconds.
    pub(crate) secs: u32,
    /// The renewal margin.
    pub(crate) margin: Duration,
}

impl Lifetime {
    /// A lifetime of `secs`, renewed
    /// [`renewal_margin`](bacnet_ops::watch::renewal_margin) before its end.
    pub(crate) fn new(secs: u32, request_budget: Duration) -> Self {
        Self {
            secs,
            margin: bacnet_ops::watch::renewal_margin(secs, request_budget),
        }
    }
}

/// Why a row is polled rather than subscribed.
#[derive(Debug)]
pub(crate) struct Refusal {
    /// Shown in the row.
    pub(crate) reason: String,
    /// The subscription may exist on the device anyway (the request timed
    /// out), so removing the row or quitting still sends a cancel.
    pub(crate) maybe_subscribed: bool,
    /// A later start may recover from this failure; independent of whether
    /// a subscription might already exist.
    pub(crate) retryable: bool,
    /// Nothing can watch it: the row goes.
    pub(crate) fatal: bool,
}

/// How a row started.
pub(crate) struct Started {
    /// The first value, with the format the row's later values take.
    pub(crate) first: Option<(Reading, RowFormat)>,
    /// The object's Object_Name, when the first read returned it.
    pub(crate) name: Option<String>,
    /// Where the device is, to subscribe and cancel; `None` when it isn't
    /// in the device table.
    pub(crate) target: Option<CovTarget>,
    /// The subscription, or why there is none.
    pub(crate) outcome: Result<ManagedCOVSubscription, Refusal>,
    /// The device refused RPM.
    pub(crate) refused_rpm: bool,
}

/// Read the point once, then subscribe for `lifetime`.
pub(crate) async fn start<T: TransportPort + 'static>(
    client: &Arc<BACnetClient<T>>,
    rate: &RequestLimits,
    row: RowId,
    point: Point,
    lifetime: Lifetime,
    rpm: bool,
) -> Started {
    let Some(device) = client.get_device(point.device).await else {
        return Started {
            first: None,
            name: None,
            target: None,
            outcome: Err(Refusal {
                reason: format!("device {} is not in the device table", point.device),
                maybe_subscribed: false,
                retryable: false,
                fatal: true,
            }),
            refused_rpm: false,
        };
    };
    let cancel = AtomicBool::new(false);
    let mut reader = Reader {
        client,
        device: point.device,
        rate,
        cancel: &cancel,
        rpm,
    };
    let unnamed = [
        point.property,
        PropertyIdentifier::STATUS_FLAGS,
        PropertyIdentifier::UNITS,
    ];
    let read = first_read(&mut reader, point.object, &unnamed, true, true).await;
    let name = match &read {
        Ok(rows) => bacnet_ops::format::object_name(rows),
        Err(_) => None,
    };
    let first = match &read {
        Ok(rows) => sample::from_rows(point.property, rows)
            .map(|reading| (reading, RowFormat::from_rows(rows))),
        Err(_) => None,
    };
    let refused_rpm = rpm && !reader.rpm;
    let target = CovTarget::of_device(&device);
    let outcome = match read {
        // Subscribing would only time out too.
        Err(Halt::Device(error)) if unreachable(&error) => Err(Refusal {
            reason: format!("no answer: {error}"),
            maybe_subscribed: false,
            retryable: true,
            fatal: false,
        }),
        _ => subscribe(client, rate, &target, row, point, lifetime).await,
    };
    Started {
        first,
        name,
        target: Some(target),
        outcome,
        refused_rpm,
    }
}

/// SubscribeCOV for Present_Value, SubscribeCOVProperty for anything else,
/// both with unconfirmed notifications and a finite lifetime.
async fn subscribe<T: TransportPort + 'static>(
    client: &Arc<BACnetClient<T>>,
    rate: &RequestLimits,
    target: &CovTarget,
    row: RowId,
    point: Point,
    lifetime: Lifetime,
) -> Result<ManagedCOVSubscription, Refusal> {
    let options = ManagedCOVSubscriptionOptions::default()
        .with_renewal_margin(lifetime.margin)
        .with_renewal_gate(rate.gate(point.device));
    // The first request goes under this turn; the renewals take the gate
    // themselves.
    let _turn = rate.turn(point.device).await;
    let managed = if point.whole_object() {
        Arc::clone(client)
            .manage_cov_subscription(
                target.clone(),
                row,
                point.object,
                false,
                lifetime.secs,
                options,
            )
            .await
    } else {
        let subscription = CovPropertySubscription {
            subscriber_process_identifier: row,
            monitored_object_identifier: point.object,
            monitored_property_identifier: point.property,
            monitored_property_array_index: None,
            confirmed: false,
            lifetime: NonZeroU32::new(lifetime.secs).unwrap_or(NonZeroU32::MIN),
            cov_increment: None,
        };
        Arc::clone(client)
            .manage_cov_property_subscription(target.clone(), subscription, options)
            .await
    };
    managed.map_err(|error| Refusal {
        maybe_subscribed: unreachable(&error),
        retryable: unreachable(&error),
        reason: format!("COV refused: {error}"),
        fatal: false,
    })
}

/// Cancel row `row`'s subscription: stop renewing it, then send a
/// subscription with neither a lifetime nor the confirmed flag, which the
/// device takes as a cancellation (Clauses 13.14.2 and 13.15.2). It goes
/// to `target`, the way the subscription went, so a device the table has
/// since dropped or moved is still the one cancelled.
pub(crate) async fn cancel<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RequestLimits,
    row: RowId,
    point: Point,
    target: CovTarget,
    managed: Option<ManagedCOVSubscription>,
) -> Result<(), Error> {
    // The turn first, then the stop. A renewal in flight holds the gate, so
    // by the time the turn comes none is: the stop only ends a wait (for
    // the renewal's time or for the gate), never a request. Aborting a
    // routed renewal mid-request would quarantine the path through its
    // router for a whole request budget, longer than the session's cancel
    // wait, and every cancel through that router would give up.
    let _turn = rate.turn(point.device).await;
    if let Some(managed) = managed {
        managed.stop().await;
    }
    if point.whole_object() {
        client
            .unsubscribe_cov_target(&target, row, point.object)
            .await
    } else {
        client
            .unsubscribe_cov_property_target(&target, row, point.object, point.property, None)
            .await
    }
}

/// One poll of a device's due rows.
pub(crate) struct Polled {
    /// A reading for every row asked for: its value, or why it has none.
    pub(crate) readings: Vec<(RowId, Reading)>,
    /// The device refused RPM during this poll.
    pub(crate) refused_rpm: bool,
    /// The device stopped answering: the poll gave up on the rows left.
    pub(crate) unreachable: bool,
}

/// Read every row in `rows` of `device`, each with its Status_Flags, by
/// [`poll::poll`]: once the device has refused RPM, only the watched
/// property is read.
pub(crate) async fn poll<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RequestLimits,
    device: u32,
    rows: Vec<(RowId, Point)>,
    rpm: bool,
) -> Polled {
    let asked: Vec<PollRow<(RowId, PropertyIdentifier)>> = rows
        .iter()
        .map(|(row, point)| PollRow {
            key: (*row, point.property),
            object: point.object,
            wanted: vec![
                (point.property, None),
                (PropertyIdentifier::STATUS_FLAGS, None),
            ],
            one_by_one: 1,
        })
        .collect();
    let polled = poll::poll(client, rate, device, &asked, rpm).await;
    let readings = polled
        .rows
        .into_iter()
        .map(|((row, property), read)| {
            let reading = match read {
                Ok(rows) => sample::from_rows(property, &rows)
                    .unwrap_or_else(|| Reading::failed(poll::MISSING.into())),
                Err(error) => Reading::failed(error),
            };
            (row, reading)
        })
        .collect();
    Polled {
        readings,
        refused_rpm: polled.refused_rpm,
        unreachable: polled.unreachable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_margin_covers_a_gate_wait_and_a_renewal_within_half_the_lifetime() {
        let budget = Duration::from_secs(24);
        assert_eq!(Lifetime::new(300, budget).margin, budget * 2);
        assert_eq!(
            Lifetime::new(600, Duration::from_secs(10)).margin,
            Duration::from_secs(30)
        );
        assert_eq!(Lifetime::new(120, budget).margin, budget * 2);
        assert_eq!(Lifetime::new(80, budget).margin, Duration::from_secs(40));
        assert_eq!(Lifetime::new(30, budget).margin, Duration::from_secs(15));
        assert_eq!(
            Lifetime::new(60, Duration::ZERO).margin,
            Duration::from_secs(6)
        );
    }
}

#[cfg(test)]
#[path = "job_retry_tests.rs"]
mod retry_tests;

#[cfg(test)]
#[path = "margin_tests.rs"]
mod margin_tests;
