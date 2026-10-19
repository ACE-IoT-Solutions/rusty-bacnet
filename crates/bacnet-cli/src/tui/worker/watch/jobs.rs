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

use super::sample::{self, Reading, RowFormat};
use crate::tui::format::{RawRow, StateTexts};
use crate::tui::watched::{Point, RowId};
use crate::tui::worker::browse::limits::RateCap;
use crate::tui::worker::browse::read::{
    rpm_failure, unreachable, Halt, Reader, RpmFailure, Wanted,
};

/// Rows read by one ReadPropertyMultiple when polling.
pub(crate) const MAX_BATCH: usize = 16;

/// A subscription's lifetime and how long before its end it is renewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lifetime {
    /// Seconds.
    pub(crate) secs: u32,
    /// The renewal margin.
    pub(crate) margin: Duration,
}

impl Lifetime {
    /// A lifetime of `secs`, renewed a tenth of it early (1 to 30 s), but
    /// never later than `request_budget` before the end, the time a renewal
    /// takes with every retry, so one lost packet can't let it lapse. The
    /// margin stays at most half the lifetime.
    pub(crate) fn new(secs: u32, request_budget: Duration) -> Self {
        let tenth = Duration::from_secs(u64::from(secs / 10).clamp(1, 30));
        let half = Duration::from_secs(u64::from(secs)) / 2;
        Self {
            secs,
            margin: tenth.max(request_budget).min(half),
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
    /// Nothing can watch it: the row goes.
    pub(crate) fatal: bool,
}

/// How a row started.
pub(crate) struct Started {
    /// The first value, with the format the row's later values take.
    pub(crate) first: Option<(Reading, RowFormat)>,
    /// The device's MAC, for a device on this network.
    pub(crate) mac: Option<MacAddr>,
    /// The subscription, or why there is none.
    pub(crate) outcome: Result<ManagedCOVSubscription, Refusal>,
    /// The device refused RPM.
    pub(crate) refused_rpm: bool,
}

/// Read the point once, then subscribe for `lifetime`.
pub(crate) async fn start<T: TransportPort + 'static>(
    client: &Arc<BACnetClient<T>>,
    rate: &RateCap,
    row: RowId,
    point: Point,
    lifetime: Lifetime,
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
    // The value, its flags and Units, and a binary or multi-state object's
    // state texts, which name its later values too.
    let mut wanted = vec![
        point.property,
        PropertyIdentifier::STATUS_FLAGS,
        PropertyIdentifier::UNITS,
    ];
    wanted.extend_from_slice(StateTexts::properties(point.object.object_type()));
    let read = reader.properties(point.object, Wanted::List(&wanted)).await;
    let refused_rpm = rpm && !reader.rpm;
    let first = match &read {
        Ok((rows, _)) => {
            sample::from_rows(point.property, rows).map(|r| (r, RowFormat::from_rows(rows)))
        }
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
    lifetime: Lifetime,
) -> Result<ManagedCOVSubscription, Refusal> {
    let options = ManagedCOVSubscriptionOptions::default().with_renewal_margin(lifetime.margin);
    rate.acquire().await;
    let managed = if point.whole_object() {
        Arc::clone(client)
            .manage_cov_subscription(mac, row, point.object, false, lifetime.secs, options)
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
    /// A reading for every row asked for: its value, or why it has none.
    pub(crate) readings: Vec<(RowId, Reading)>,
    /// The device refused RPM during this poll.
    pub(crate) refused_rpm: bool,
    /// The device stopped answering: the poll gave up on the rows left.
    pub(crate) unreachable: bool,
}

/// Read every row in `rows` of `device`: by ReadPropertyMultiple in
/// batches of [`MAX_BATCH`], each row with its Status_Flags, or by
/// ReadProperty once the device has refused RPM (Clause 15.5). A batch the
/// device fails as a whole is read by ReadProperty for this poll, so one
/// bad object can't put errors on its neighbours. A device that stops
/// answering ends the poll: the rows left get the error at once.
pub(crate) async fn poll<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RateCap,
    device: u32,
    rows: Vec<(RowId, Point)>,
    mut rpm: bool,
) -> Polled {
    let started_rpm = rpm;
    let mut samples = Vec::with_capacity(rows.len());
    let mut silent: Option<Error> = None;
    for batch in rows.chunks(MAX_BATCH) {
        if let Some(error) = &silent {
            samples.extend(batch.iter().map(|(row, _)| (*row, failed(error))));
            continue;
        }
        if rpm {
            match rpm_batch(client, rate, device, batch).await {
                Ok(read) => {
                    samples.extend(read);
                    continue;
                }
                Err(error) if unreachable(&error) => {
                    samples.extend(batch.iter().map(|(row, _)| (*row, failed(&error))));
                    silent = Some(error);
                    continue;
                }
                Err(error) => match rpm_failure(&error) {
                    RpmFailure::Refused => {
                        tracing::info!("device {device} refused ReadPropertyMultiple ({error}); polling one property at a time");
                        rpm = false;
                    }
                    RpmFailure::TooLarge | RpmFailure::Other => tracing::debug!(
                        "device {device}: batch read failed ({error}); reading it one property at a time"
                    ),
                },
            }
        }
        for (row, point) in batch {
            if let Some(error) = &silent {
                samples.push((*row, failed(error)));
                continue;
            }
            rate.acquire().await;
            let value = match client
                .read_property_from_device(device, point.object, point.property, None)
                .await
            {
                Ok(ack) => Ok(ack.property_value),
                Err(error) if unreachable(&error) => {
                    samples.push((*row, failed(&error)));
                    silent = Some(error);
                    continue;
                }
                Err(error) => Err(error.to_string()),
            };
            let rows: Vec<RawRow> = vec![(point.property, value)];
            if let Some(reading) = sample::from_rows(point.property, &rows) {
                samples.push((*row, reading));
            }
        }
    }
    Polled {
        readings: samples,
        refused_rpm: started_rpm && !rpm,
        unreachable: silent.is_some(),
    }
}

fn failed(error: &Error) -> Reading {
    failed_with(error.to_string())
}

fn failed_with(error: String) -> Reading {
    Reading::failed(error)
}

/// One ReadPropertyMultiple for `batch`, one access specification per row.
async fn rpm_batch<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RateCap,
    device: u32,
    batch: &[(RowId, Point)],
) -> Result<Vec<(RowId, Reading)>, Error> {
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
    // The answer lists the objects in the order they were asked for; a row
    // it leaves out, or answers without the property, gets an error.
    let mut results = ack.list_of_read_access_results.into_iter();
    Ok(batch
        .iter()
        .map(|(row, point)| {
            let Some(result) = results.next() else {
                return (*row, failed_with("missing from the device's answer".into()));
            };
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
            let sample = sample::from_rows(point.property, &rows)
                .unwrap_or_else(|| failed_with("missing from the device's answer".into()));
            (*row, sample)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_margin_covers_a_renewal_with_its_retries_within_half_the_lifetime() {
        let budget = Duration::from_secs(24);
        assert_eq!(Lifetime::new(300, budget).margin, Duration::from_secs(30));
        assert_eq!(Lifetime::new(120, budget).margin, budget);
        assert_eq!(Lifetime::new(30, budget).margin, Duration::from_secs(15));
        assert_eq!(
            Lifetime::new(60, Duration::ZERO).margin,
            Duration::from_secs(6)
        );
    }
}
