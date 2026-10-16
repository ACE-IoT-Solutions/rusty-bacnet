//! Each port's sender: the task that owns the port's transport and carries
//! out the sends queued for it.
//!
//! A send the transport fails is logged with the port's throttled warning
//! ([`send_warning`](super::send_warning)). One that is longer than the
//! limit the transport reports now is counted too, in the engine's
//! `egress_too_long` and `refused` (#1696): a send queued while the port
//! still carried more, before the router heard that its limit fell, as a
//! BACnet/SC port's can on failover to a hub that carries less. The engine
//! would have answered it with reject reason 4 had it known; this late, the
//! send is only counted.

use std::sync::Arc;

use bacnet_router_core::router::SendRefusal;
use bacnet_transport::port::TransportPort;
use tokio::sync::mpsc;

use super::dispatch::Shared;
use super::send_warning::SendWarnings;
use super::SendRequest;

/// Carry out every send on `queue` on `transport`, until the queue closes.
pub(super) async fn run<T: TransportPort>(
    transport: T,
    mut queue: mpsc::Receiver<SendRequest>,
    shared: Arc<Shared>,
) {
    let warnings = SendWarnings::default();
    while let Some(request) = queue.recv().await {
        match request {
            SendRequest::Unicast {
                npdu,
                mac,
                data_attributes,
            } => {
                if let Err(error) = transport
                    .send_unicast_with_data_attributes(&npdu, &mac, &data_attributes)
                    .await
                {
                    count_if_too_long(&shared, &transport, npdu.len(), false);
                    warnings.unicast_failed(&error);
                }
            }
            SendRequest::Broadcast {
                npdu,
                data_attributes,
            } => {
                if let Err(error) = transport
                    .send_broadcast_with_data_attributes(&npdu, &data_attributes)
                    .await
                {
                    count_if_too_long(&shared, &transport, npdu.len(), true);
                    warnings.broadcast_failed(&error);
                }
            }
        }
    }
}

/// Count a failed send of `octets` with the engine's late refusals when it
/// is longer than what `transport` now carries for its kind of send.
fn count_if_too_long<T: TransportPort>(
    shared: &Shared,
    transport: &T,
    octets: usize,
    broadcast: bool,
) {
    let limits = transport.max_npdu();
    let limit = if broadcast {
        limits.broadcast
    } else {
        limits.unicast
    };
    if octets > usize::from(limit) {
        shared.engine.lock().refused_later(SendRefusal::TooLong);
    }
}
