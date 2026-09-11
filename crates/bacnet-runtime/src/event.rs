use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, Notify};

use bacnet_client::client::DeviceEvent;
use bacnet_client::client::{COVNotificationDelivery, ReceivedCOVNotification};
use bacnet_types::enums::Segmentation;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;

use crate::{AttachmentId, CovValue, ErrorCode, ObservationKey};

/// One losslessly preserved property value from an unsolicited COV notification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedCovValue {
    /// Numeric BACnet property identifier.
    pub property_id: u32,
    /// Optional BACnet array index.
    pub array_index: Option<u32>,
    /// Complete application-tagged value bytes.
    pub raw_value: Vec<u8>,
    /// Optional BACnet write priority carried by the value.
    pub priority: Option<u8>,
}

/// Lossless unsolicited COV payload exposed through the aggregate runtime stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsolicitedCovNotification {
    /// Subscriber process identifier supplied by the sender.
    pub subscriber_process_identifier: u32,
    /// Initiating Device object identifier supplied by the sender.
    pub initiating_device_identifier: ObjectIdentifier,
    /// Monitored object identifier supplied by the sender.
    pub monitored_object_identifier: ObjectIdentifier,
    /// Remaining subscription lifetime reported by the sender.
    pub time_remaining: u32,
    /// Whether the service was confirmed or unconfirmed.
    pub delivery: COVNotificationDelivery,
    /// Immediate data-link source MAC, preserved byte-for-byte.
    pub source_mac: MacAddr,
    /// NPDU SNET, when present.
    pub source_network: Option<u16>,
    /// NPDU SADR, when present, preserved byte-for-byte.
    pub source_address: Option<MacAddr>,
    /// Values in notification order, including raw application tags.
    pub values: Vec<ObservedCovValue>,
}

impl From<&ReceivedCOVNotification> for UnsolicitedCovNotification {
    fn from(received: &ReceivedCOVNotification) -> Self {
        let notification = &received.notification;
        Self {
            subscriber_process_identifier: notification.subscriber_process_identifier,
            initiating_device_identifier: notification.initiating_device_identifier,
            monitored_object_identifier: notification.monitored_object_identifier,
            time_remaining: notification.time_remaining,
            delivery: received.delivery,
            source_mac: received.source_mac.clone(),
            source_network: received.source_network,
            source_address: received.source_address.clone(),
            values: notification
                .list_of_values
                .iter()
                .map(|value| ObservedCovValue {
                    property_id: value.property_identifier.to_raw(),
                    array_index: value.property_array_index,
                    raw_value: value.value.clone(),
                    priority: value.priority,
                })
                .collect(),
        }
    }
}

/// I-Am-derived device observation exposed through the aggregate runtime stream.
///
/// Upstream 0.11 supplies the authoritative discovery-table snapshot but not
/// BVLL datagram metadata; fields requiring that raw envelope are `None`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IAmObservation {
    /// Device instance extracted from `object_identifier`.
    pub device_instance: u32,
    /// Device object identifier carried by the I-Am.
    pub object_identifier: ObjectIdentifier,
    /// Maximum APDU length accepted by the device.
    pub max_apdu_length: u32,
    /// Segmentation capability advertised by the device.
    pub segmentation_supported: Segmentation,
    /// Vendor identifier advertised by the device.
    pub vendor_id: u16,
    /// Actual UDP sender IPv4 address for BACnet/IP, otherwise `None`.
    pub udp_source_ip: Option<[u8; 4]>,
    /// Actual UDP sender port for BACnet/IP, otherwise `None`.
    pub udp_source_port: Option<u16>,
    /// Immediate data-link source MAC, preserved byte-for-byte.
    pub source_mac: MacAddr,
    /// NPDU SNET, when present.
    pub source_network: Option<u16>,
    /// NPDU SADR, when present, preserved byte-for-byte.
    pub source_address: Option<MacAddr>,
    /// Raw BVLC function code for BACnet/IP, otherwise `None`.
    pub bvlc_function: Option<u8>,
    /// Forwarded-NPDU originator IPv4 address, when present.
    pub forwarded_from_ip: Option<[u8; 4]>,
    /// Forwarded-NPDU originator UDP port, when present.
    pub forwarded_from_port: Option<u16>,
    /// Monotonic observation timestamp in this process.
    pub timestamp: std::time::Instant,
}

impl From<DeviceEvent> for IAmObservation {
    fn from(event: DeviceEvent) -> Self {
        let device = event.device;
        Self {
            device_instance: device.object_identifier.instance_number(),
            object_identifier: device.object_identifier,
            max_apdu_length: device.max_apdu_length,
            segmentation_supported: device.segmentation_supported,
            vendor_id: device.vendor_id,
            udp_source_ip: None,
            udp_source_port: None,
            source_mac: device.mac_address,
            source_network: device.source_network,
            source_address: device.source_address,
            bvlc_function: None,
            forwarded_from_ip: None,
            forwarded_from_port: None,
            timestamp: device.last_seen,
        }
    }
}

/// Coarse runtime event kind.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EventKind {
    /// Runtime started with its initial generation.
    RuntimeStarted,
    /// Runtime stopped and released attachment ownership.
    RuntimeStopped,
    /// One attachment changed lifecycle state.
    AttachmentStateChanged,
    /// I-Am-derived device observation received by one attachment.
    ///
    /// Discovery and refresh events remain independently visible. Upstream's
    /// device-event boundary does not expose rejected duplicate endpoint claims.
    IAmObservation {
        /// Lossless client observation, including NPDU and BVLL provenance.
        observation: IAmObservation,
    },
    /// COV notification that did not match a runtime-managed subscription.
    UnsolicitedCovNotification {
        /// Lossless notification payload and transport/NPDU provenance.
        notification: UnsolicitedCovNotification,
    },
    /// Coarse scan progress, rate-limited except for the final update.
    ScanProgress {
        /// BACnet Device object instance being scanned.
        device_instance: u32,
        /// Completed RPM-plan batches.
        completed_batches: usize,
        /// Total RPM-plan batches.
        total_batches: usize,
        /// True for the mandatory final update.
        final_update: bool,
    },
    /// Deduplicated COV notification with raw property values.
    CovNotification {
        /// Managed subscription that matched the notification.
        key: ObservationKey,
        /// Remaining subscription lifetime reported by the device.
        time_remaining: u32,
        /// Raw values in notification order.
        values: Vec<CovValue>,
    },
    /// Managed subscription renewed successfully.
    CovRenewed {
        /// Renewed subscription.
        key: ObservationKey,
    },
    /// Renewal failed and will be retried while the plan remains active.
    CovRenewalFailed {
        /// Subscription whose renewal failed.
        key: ObservationKey,
        /// Stable failure category.
        code: ErrorCode,
    },
    /// Attachment COV receiver lost notifications to broadcast backpressure.
    CovNotificationLagged {
        /// Number of skipped notifications reported by the receiver.
        skipped: u64,
    },
    /// Attachment I-Am receiver lost observations to broadcast backpressure.
    IAmObservationLagged {
        /// Number of skipped observations reported by the receiver.
        skipped: u64,
    },
    /// Events were evicted from the bounded queue.
    EventLagged {
        /// First lost sequence number.
        first_lost: u64,
        /// Last lost sequence number.
        last_lost: u64,
        /// Number of lost events.
        count: u64,
    },
}

/// Immutable event delivered over the coarse runtime boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeEvent {
    /// Runtime generation that emitted the event.
    pub generation: u64,
    /// Monotonically increasing sequence number.
    pub sequence: u64,
    /// Optional attachment provenance.
    pub attachment_id: Option<AttachmentId>,
    /// Event payload.
    pub kind: EventKind,
}

/// One bounded retrieval from the runtime event stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventBatch {
    /// Events ordered by sequence.
    pub events: Vec<RuntimeEvent>,
}

#[derive(Debug)]
struct EventState {
    queue: VecDeque<RuntimeEvent>,
    first_lost: Option<u64>,
    last_lost: u64,
    lost_count: u64,
}

#[derive(Debug)]
pub(crate) struct EventBus {
    capacity: usize,
    max_batch: usize,
    next_sequence: AtomicU64,
    total_lost: AtomicU64,
    state: Mutex<EventState>,
    notify: Notify,
}

impl EventBus {
    pub(crate) fn new(capacity: usize, max_batch: usize) -> Self {
        Self {
            capacity,
            max_batch,
            next_sequence: AtomicU64::new(1),
            total_lost: AtomicU64::new(0),
            state: Mutex::new(EventState {
                queue: VecDeque::with_capacity(capacity),
                first_lost: None,
                last_lost: 0,
                lost_count: 0,
            }),
            notify: Notify::new(),
        }
    }

    pub(crate) async fn publish(
        &self,
        generation: u64,
        attachment_id: Option<AttachmentId>,
        kind: EventKind,
    ) {
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        let mut state = self.state.lock().await;
        if state.queue.len() == self.capacity {
            if let Some(lost) = state.queue.pop_front() {
                state.first_lost.get_or_insert(lost.sequence);
                state.last_lost = lost.sequence;
                state.lost_count += 1;
                self.total_lost.fetch_add(1, Ordering::Relaxed);
            }
        }
        state.queue.push_back(RuntimeEvent {
            generation,
            sequence,
            attachment_id,
            kind,
        });
        drop(state);
        self.notify.notify_waiters();
    }

    pub(crate) async fn next_batch(&self, requested: usize, wait: Duration) -> EventBatch {
        let limit = requested.clamp(1, self.max_batch);
        loop {
            let mut state = self.state.lock().await;
            if state.first_lost.is_some() || !state.queue.is_empty() {
                let mut events = Vec::with_capacity(limit);
                while events.len() < limit {
                    let Some(event) = state.queue.pop_front() else {
                        break;
                    };
                    events.push(event);
                }
                if events.len() < limit {
                    if let Some(first_lost) = state.first_lost.take() {
                        let last_lost = state.last_lost;
                        let count = state.lost_count;
                        state.lost_count = 0;
                        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
                        events.push(RuntimeEvent {
                            generation: 0,
                            sequence,
                            attachment_id: None,
                            kind: EventKind::EventLagged {
                                first_lost,
                                last_lost,
                                count,
                            },
                        });
                    }
                }
                return EventBatch { events };
            }
            drop(state);
            if tokio::time::timeout(wait, self.notify.notified())
                .await
                .is_err()
            {
                return EventBatch { events: Vec::new() };
            }
        }
    }

    pub(crate) async fn queue_depth(&self) -> usize {
        self.state.lock().await.queue.len()
    }

    pub(crate) async fn lag_count(&self) -> u64 {
        self.total_lost.load(Ordering::Relaxed)
    }

    pub(crate) async fn clear(&self) {
        let mut state = self.state.lock().await;
        state.queue.clear();
        state.first_lost = None;
        state.last_lost = 0;
        state.lost_count = 0;
        self.total_lost.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{EventBus, EventKind};

    #[tokio::test]
    async fn bounded_queue_reports_exact_loss_without_reordering_sequences() {
        let bus = EventBus::new(2, 3);
        for _ in 0..4 {
            bus.publish(7, None, EventKind::AttachmentStateChanged)
                .await;
        }

        let batch = bus.next_batch(3, Duration::ZERO).await;
        assert_eq!(
            batch
                .events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
        assert_eq!(
            batch.events[2].kind,
            EventKind::EventLagged {
                first_lost: 1,
                last_lost: 2,
                count: 2,
            }
        );
        assert_eq!(bus.lag_count().await, 2);
        assert_eq!(bus.queue_depth().await, 0);
    }

    #[tokio::test]
    async fn empty_event_read_observes_wait_timeout() {
        let bus = EventBus::new(2, 2);
        assert!(bus
            .next_batch(2, Duration::from_millis(1))
            .await
            .events
            .is_empty());
    }
}
