//! A log page's requests, each charged to the limits.
//!
//! One page of a Trend Log, Trend Log Multiple, Event Log or Audit Log can
//! take several requests: the log's record counts, then the ReadRange, and
//! the counts again when the log moved meanwhile. The page holds its
//! device's turn throughout, so they still go one at a time; the turn pays
//! for the first request and each later one takes a slot of its own, so a
//! page costs what it sends (#2066).
//!
//! Once a Trend Log's first page is answered, [`Engine::log_columns`]
//! reads what it samples, so its values can be named (#2091): one more
//! request, charged the same way, once per read rather than once per page.
//! Asking only after an answer keeps a silent device to one timeout.

use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_client::client::BACnetClient;
use bacnet_client::log_reader::{LogRequester, unsigned_value};
use bacnet_services::read_range::{ReadRangeReply, ReadRangeRequest, ReadRangeValidation};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::Engine;
use crate::format::LogColumns;
use crate::limits::RequestLimits;

/// The requester a page reads through: the device's routed read calls,
/// each after a slot (the first under the page's turn).
pub(super) struct PagedLog<'a, T: TransportPort + 'static> {
    pub(super) client: &'a BACnetClient<T>,
    pub(super) limits: &'a RequestLimits,
    pub(super) device: u32,
    pub(super) network: Option<u16>,
    /// Set until the first request, which the page's turn paid for.
    pub(super) first: AtomicBool,
}

impl<T: TransportPort + 'static> PagedLog<'_, T> {
    async fn charge(&self) {
        if !self.first.swap(false, Ordering::Relaxed) {
            self.limits.slot(self.network).await;
        }
    }
}

impl<T: TransportPort + 'static> LogRequester for PagedLog<'_, T> {
    async fn read_unsigned(
        &self,
        _mac: &[u8],
        log: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> Result<u64, Error> {
        self.charge().await;
        let ack = self
            .client
            .read_property_from_device(self.device, log, property, None)
            .await?;
        unsigned_value(&ack.property_value)
    }

    async fn read_range_lenient(
        &self,
        _mac: &[u8],
        request: &ReadRangeRequest,
    ) -> Result<ReadRangeReply, Error> {
        self.charge().await;
        self.client
            .read_range_from_device_with(self.device, request, ReadRangeValidation::Lenient)
            .await
    }
}

impl<T: TransportPort + 'static> Engine<T> {
    /// What each column of `log` on `device` samples, from its
    /// Log_DeviceObjectProperty: one ReadProperty, charged like any other
    /// read, for a Trend Log or Trend Log Multiple, and none for another
    /// log. Naming is best effort: a refused, failed or empty read names no
    /// column, and the values print as the log gives them.
    pub async fn log_columns(&self, device: u32, log: ObjectIdentifier) -> LogColumns {
        if !LogColumns::applies(log.object_type()) {
            return LogColumns::default();
        }
        self.read_property(
            device,
            log,
            PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY,
            None,
        )
        .await
        .map(|data| LogColumns::decode(&data))
        .unwrap_or_default()
    }
}
