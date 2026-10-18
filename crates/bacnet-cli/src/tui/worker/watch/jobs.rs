//! The watch list's requests to one device: starting a row (a first read,
//! then a managed COV subscription), cancelling one, and polling a batch.
//!
//! Each runs as one job, and a device runs one job at a time, so the watch
//! list never has more than one confirmed request outstanding to a device.
//! Every request first takes a slot from the shared [`RateCap`]. The
//! managed subscription's own renewals are sent by its task, outside both
//! limits, once per lifetime.

use std::num::NonZeroU32;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::{
    BACnetClient, CovPropertySubscription, ManagedCOVSubscription, ManagedCOVSubscriptionOptions,
};
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::MacAddr;

use super::sample;
use crate::tui::watched::{Point, RowId, Sample};
use crate::tui::worker::browse::format::RawRow;
use crate::tui::worker::browse::limits::RateCap;
use crate::tui::worker::browse::read::{
    rpm_failure, unreachable, Halt, Reader, RpmFailure, Wanted,
};

/// Rows read by one ReadPropertyMultiple when polling.
pub(crate) const MAX_BATCH: usize = 16;

/// How long before a lifetime ends the subscription is renewed: a tenth of
/// it, between 1 and 30 seconds.
pub(crate) fn renewal_margin(lifetime: u32) -> Duration {
    Duration::from_secs(u64::from(lifetime / 10).clamp(1, 30))
}

/// Why a row is polled rather than subscribed.
#[derive(Debug)]
pub(crate) struct Refusal {
    /// Shown in the row.
    pub(crate) reason: String,
    /// The subscription may exist on the device anyway (the request timed
    /// out), so removing the row or quitting still sends a cancel.
    pub(crate) maybe_subscribed: bool,
    /// Nothing can watch it: the row goes.
    pub(crate) fatal: bool,
}

/// How a row started.
pub(crate) struct Started {
    /// The first value, with Units.
    pub(crate) first: Option<(Sample, Option<String>)>,
    /// The device's MAC, for a device on this network.
    pub(crate) mac: Option<MacAddr>,
    /// The subscription, or why there is none.
    pub(crate) outcome: Result<ManagedCOVSubscription, Refusal>,
    /// The device refused RPM.
    pub(crate) refused_rpm: bool,
}

/// Read the point once, then subscribe for `lifetime` seconds.
pub(crate) async fn start<T: TransportPort + 'static>(
    client: &Arc<BACnetClient<T>>,
    rate: &RateCap,
    row: RowId,
    point: Point,
    lifetime: u32,
    rpm: bool,
) -> Started {
    let Some(device) = client.get_device(point.device).await else {
        return Started {
            first: None,
            mac: None,
            outcome: Err(Refusal {
                reason: format!("device {} is not in the device table", point.device),
                maybe_subscribed: false,
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
    let wanted = [
        point.property,
        PropertyIdentifier::STATUS_FLAGS,
        PropertyIdentifier::UNITS,
    ];
    let read = reader.properties(point.object, Wanted::List(&wanted)).await;
    let refused_rpm = rpm && !reader.rpm;
    let first = match &read {
        Ok((rows, _)) => sample::from_rows(point.property, rows).map(|s| (s, sample::units(rows))),
        Err(_) => None,
    };
    let outcome = match read {
        // Subscribing would only time out too.
        Err(Halt::Device(error)) if unreachable(&error) => Err(Refusal {
            reason: format!("no answer: {error}"),
            maybe_subscribed: false,
            fatal: false,
        }),
        _ if device.source_network.is_some() => Err(Refusal {
            reason: "routed device".into(),
            maybe_subscribed: false,
            fatal: false,
        }),
        _ => {
            subscribe(
                client,
                rate,
                device.mac_address.as_slice(),
                row,
                point,
                lifetime,
            )
            .await
        }
    };
    let mac = device
        .source_network
        .is_none()
        .then_some(device.mac_address);
    Started {
        first,
        mac,
        outcome,
        refused_rpm,
    }
}

/// SubscribeCOV for Present_Value, SubscribeCOVProperty for anything else,
/// both with unconfirmed notifications and a finite lifetime.
async fn subscribe<T: TransportPort + 'static>(
    client: &Arc<BACnetClient<T>>,
    rate: &RateCap,
    mac: &[u8],
    row: RowId,
    point: Point,
    lifetime: u32,
) -> Result<ManagedCOVSubscription, Refusal> {
    let options =
        ManagedCOVSubscriptionOptions::default().with_renewal_margin(renewal_margin(lifetime));
    rate.acquire().await;
    let managed = if point.whole_object() {
        Arc::clone(client)
            .manage_cov_subscription(mac, row, point.object, false, lifetime, options)
            .await
    } else {
        let subscription = CovPropertySubscription {
            subscriber_process_identifier: row,
            monitored_object_identifier: point.object,
            monitored_property_identifier: point.property,
            monitored_property_array_index: None,
            confirmed: false,
            lifetime: NonZeroU32::new(lifetime).unwrap_or(NonZeroU32::MIN),
            cov_increment: None,
        };
        Arc::clone(client)
            .manage_cov_property_subscription(mac, subscription, options)
            .await
    };
    managed.map_err(|error| Refusal {
        maybe_subscribed: unreachable(&error),
        reason: format!("COV refused: {error}"),
        fatal: false,
    })
}

/// Cancel row `row`'s subscription: stop renewing it, then send a
/// subscription with neither a lifetime nor the confirmed flag, which the
/// device takes as a cancellation (Clauses 13.14.2 and 13.15.2).
pub(crate) async fn cancel<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RateCap,
    row: RowId,
    point: Point,
    mac: MacAddr,
    managed: Option<ManagedCOVSubscription>,
) -> Result<(), Error> {
    if let Some(managed) = managed {
        managed.stop().await;
    }
    rate.acquire().await;
    if point.whole_object() {
        client.unsubscribe_cov(&mac, row, point.object).await
    } else {
        client
            .unsubscribe_cov_property(&mac, row, point.object, point.property, None)
            .await
    }
}

/// One poll of a device's due rows.
pub(crate) struct Polled {
    /// Each row's sample.
    pub(crate) samples: Vec<(RowId, Sample)>,
    /// The device refused RPM during this poll.
    pub(crate) refused_rpm: bool,
}

/// Read every row in `rows` of `device`: by ReadPropertyMultiple in
/// batches of [`MAX_BATCH`], each row with its Status_Flags, or by
/// ReadProperty once the device has refused RPM (Clause 15.5).
pub(crate) async fn poll<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RateCap,
    device: u32,
    rows: Vec<(RowId, Point)>,
    mut rpm: bool,
) -> Polled {
    let started_rpm = rpm;
    let mut samples = Vec::with_capacity(rows.len());
    for batch in rows.chunks(MAX_BATCH) {
        if rpm {
            match rpm_batch(client, rate, device, batch).await {
                Ok(read) => {
                    samples.extend(read);
                    continue;
                }
                Err(error) => match rpm_failure(&error) {
                    RpmFailure::Refused => {
                        tracing::info!("device {device} refused ReadPropertyMultiple ({error}); polling one property at a time");
                        rpm = false;
                    }
                    RpmFailure::TooLarge => {}
                    RpmFailure::Other => {
                        samples.extend(batch.iter().map(|(row, _)| (*row, failed(&error))));
                        continue;
                    }
                },
            }
        }
        for (row, point) in batch {
            rate.acquire().await;
            let value = client
                .read_property_from_device(device, point.object, point.property, None)
                .await
                .map(|ack| ack.property_value)
                .map_err(|error| error.to_string());
            let rows: Vec<RawRow> = vec![(point.property, value)];
            if let Some(sample) = sample::from_rows(point.property, &rows) {
                samples.push((*row, sample));
            }
        }
    }
    Polled {
        samples,
        refused_rpm: started_rpm && !rpm,
    }
}

fn failed(error: &Error) -> Sample {
    Sample {
        value: Err(error.to_string()),
        number: None,
        flags: None,
        at: tokio::time::Instant::now().into_std(),
    }
}

/// One ReadPropertyMultiple for `batch`, one access specification per row.
async fn rpm_batch<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RateCap,
    device: u32,
    batch: &[(RowId, Point)],
) -> Result<Vec<(RowId, Sample)>, Error> {
    let specs = batch
        .iter()
        .map(|(_, point)| ReadAccessSpecification {
            object_identifier: point.object,
            list_of_property_references: [point.property, PropertyIdentifier::STATUS_FLAGS]
                .into_iter()
                .map(|property_identifier| PropertyReference {
                    property_identifier,
                    property_array_index: None,
                })
                .collect(),
        })
        .collect();
    rate.acquire().await;
    let ack = client
        .read_property_multiple_from_device(device, specs)
        .await?;
    // The answer lists the objects in the order they were asked for.
    Ok(batch
        .iter()
        .zip(ack.list_of_read_access_results)
        .filter_map(|((row, point), result)| {
            let rows: Vec<RawRow> = result
                .list_of_results
                .into_iter()
                .map(|element| {
                    let value = match (element.property_value, element.error) {
                        (Some(value), _) => Ok(value),
                        (None, Some((class, code))) => Err(format!("{class}:{code}")),
                        (None, None) => Err("no value".into()),
                    };
                    (element.property_identifier, value)
                })
                .collect();
            sample::from_rows(point.property, &rows).map(|sample| (*row, sample))
        })
        .collect())
}
