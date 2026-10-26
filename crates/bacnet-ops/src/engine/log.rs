//! A log page's requests, each charged to the limits.
//!
//! One page of a Trend Log, Trend Log Multiple, Event Log or Audit Log can
//! take several requests: the log's record counts, then the ReadRange, and
//! the counts again when the log moved meanwhile. The page holds its
//! device's turn throughout, so they still go one at a time; the turn pays
//! for the first request and each later one takes a slot of its own, so a
//! page costs what it sends (#2066).

use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_client::client::BACnetClient;
use bacnet_client::log_reader::{LogRequester, unsigned_value};
use bacnet_services::read_range::{ReadRangeReply, ReadRangeRequest, ReadRangeValidation};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

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
