//! COV subscription command.
//!
//! A subscription is finite by default (300 seconds), renewed while the
//! command watches and cancelled when it stops, so it leaves nothing behind
//! on the device. A lifetime of 0 asks for one that never expires (Clause
//! 13.14.1.1.4); it stays after the CLI exits, which is why the write guard
//! treats it as a write (`core::class`).

use std::future::Future;
use std::time::Duration;

use bacnet_client::client::{BACnetClient, COVNotificationDelivery, ReceivedCOVNotification};
use bacnet_encoding::primitives::decode_application_value;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::broadcast::error::RecvError;

use crate::core::destination::Destination;
use crate::output::{self, OutputFormat};

/// The lifetime a subscription asks for when `--lifetime` is not given.
pub(crate) const DEFAULT_LIFETIME: u32 = 300;

/// How long before a finite subscription runs out the command renews it.
const RENEWAL_MARGIN: u32 = 30;

/// When to renew a subscription of `lifetime` seconds: `RENEWAL_MARGIN`
/// before it runs out, but never before half of it has passed, and at least
/// a second apart. `None` for an indefinite one (0), which needs none.
pub(crate) fn renewal_interval(lifetime: u32) -> Option<Duration> {
    (lifetime != 0).then(|| {
        let seconds = lifetime
            .saturating_sub(RENEWAL_MARGIN)
            .max(lifetime / 2)
            .max(1);
        Duration::from_secs(u64::from(seconds))
    })
}

/// What to subscribe to.
#[derive(Debug, Clone, Copy)]
pub struct Subscription {
    /// The object's type.
    pub object_type: ObjectType,
    /// The object's instance.
    pub instance: u32,
    /// Seconds; 0 for a subscription that never expires.
    pub lifetime: u32,
    /// Ask for confirmed notifications.
    pub confirmed: bool,
}

/// Ctrl+C, or on Unix also SIGTERM: what ends `subscribe`'s watch. On
/// Unix both handlers are installed when this is called, before the
/// subscription goes out, so an early signal still cancels it.
#[cfg(unix)]
pub(crate) fn stop_signal() -> impl Future<Output = ()> {
    use tokio::signal::unix::{signal, SignalKind};
    let interrupt = signal(SignalKind::interrupt());
    let terminate = signal(SignalKind::terminate());
    async move {
        let (Ok(mut interrupt), Ok(mut terminate)) = (interrupt, terminate) else {
            // No handler: stop at once rather than leave one we can't end.
            return;
        };
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
        }
    }
}

/// Ctrl+C, or on Unix also SIGTERM: what ends `subscribe`'s watch.
#[cfg(not(unix))]
pub(crate) async fn stop_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

/// Subscribe to COV notifications for an object on a remote device and
/// print them until `stop` completes. A finite subscription is renewed
/// meanwhile and cancelled at the end, however the watch ends.
pub async fn subscribe_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    subscription: Subscription,
    format: OutputFormat,
    stop: impl Future<Output = ()>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Subscription {
        object_type,
        instance,
        lifetime,
        confirmed,
    } = subscription;
    let oid = ObjectIdentifier::new(object_type, instance)?;
    let process_id = std::process::id(); // Use PID instead of hardcoded 1
                                         // Absent with Issue Confirmed Notifications present is indefinite too
                                         // (Clause 13.14.2), and is what the CLI has always sent for it.
    let wire_lifetime = (lifetime != 0).then_some(lifetime);

    let mut rx = client.cov_notifications();
    to.subscribe_cov(client, process_id, oid, confirmed, wire_lifetime)
        .await?;

    let kept = match lifetime {
        0 => "indefinite: it stays after the CLI exits".to_string(),
        _ => format!("lifetime {lifetime}s, renewed while watching, cancelled on exit"),
    };
    output::print_success(
        &format!(
            "Subscribed to {}:{} (process_id={}; {kept}). Watching... (Ctrl+C to stop)",
            object_type, instance, process_id
        ),
        format,
    );

    let renew_every = renewal_interval(lifetime);
    let mut renew_at = renew_every.map(|every| tokio::time::Instant::now() + every);
    tokio::pin!(stop);
    let watched: Result<(), Box<dyn std::error::Error>> = loop {
        let renew = async {
            match renew_at {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            () = &mut stop => break Ok(()),
            () = renew => {
                if let Err(error) = to
                    .subscribe_cov(client, process_id, oid, confirmed, wire_lifetime)
                    .await
                {
                    break Err(format!("renewing the subscription failed: {error}").into());
                }
                renew_at = renew_every.map(|every| tokio::time::Instant::now() + every);
            }
            result = rx.recv() => match result {
                Ok(notification) => print_cov_notification(&notification, format),
                Err(RecvError::Lagged(n)) => {
                    output::print_error(&format!("Missed {n} notifications (buffer overflow)"));
                }
                Err(RecvError::Closed) => {
                    output::print_error("Notification channel closed");
                    break Ok(());
                }
            },
        }
    };

    if lifetime != 0 {
        if let Err(error) = to.unsubscribe_cov(client, process_id, oid).await {
            let message = format!("cancelling the subscription failed: {error}");
            return Err(match watched {
                Ok(()) => message,
                Err(watch) => format!("{watch}; {message}"),
            }
            .into());
        }
    }
    watched
}

fn print_cov_notification(received: &ReceivedCOVNotification, format: OutputFormat) {
    let notification = &received.notification;
    let obj = format!(
        "{}:{}",
        notification.monitored_object_identifier.object_type(),
        notification.monitored_object_identifier.instance_number()
    );
    let device = format!(
        "device:{}",
        notification.initiating_device_identifier.instance_number()
    );
    let delivery = match received.delivery {
        COVNotificationDelivery::Confirmed => "confirmed",
        COVNotificationDelivery::Unconfirmed => "unconfirmed",
    };
    let source_mac = format_mac(&received.source_mac);
    let source_address = received
        .source_address
        .as_ref()
        .map(|address| format_mac(address));
    let routed_source = received
        .source_network
        .zip(source_address.as_ref())
        .map(|(network, address)| format!("{network}:{address}"));

    // Decode property values
    let mut props = Vec::new();
    for pv in &notification.list_of_values {
        let prop_name = format!("{}", pv.property_identifier);
        let value_str = decode_cov_value(&pv.value);
        props.push((prop_name, value_str));
    }

    match format {
        OutputFormat::Table => {
            let mut parts = vec![format!(
                "[COV] {obj} from {device}, source={source_mac}, delivery={delivery}, remaining={}s",
                notification.time_remaining,
            )];
            if let Some(routed_source) = &routed_source {
                parts.push(format!("  routed-source = {routed_source}"));
            }
            for (prop, val) in &props {
                parts.push(format!("  {prop} = {val}"));
            }
            println!("{}", parts.join("\n"));
        }
        OutputFormat::Json => {
            let prop_map: serde_json::Map<String, serde_json::Value> = props
                .into_iter()
                .map(|(k, v)| (k, serde_json::Value::String(v)))
                .collect();
            let json = serde_json::json!({
                "type": "cov",
                "object": obj,
                "device": device,
                "delivery": delivery,
                "source_mac": source_mac,
                "source_network": received.source_network,
                "source_address": source_address,
                "time_remaining": notification.time_remaining,
                "values": prop_map,
            });
            println!("{}", serde_json::to_string(&json).unwrap_or_default());
        }
    }
}

/// A MAC as hex, except a B/IPv6 VMAC, which prints as the `bip6:` target
/// the CLI reads back. Six octets stay hex: on BACnet/SC they are a VMAC,
/// not an IPv4 address and port.
fn format_mac(mac: &[u8]) -> String {
    if let Ok(vmac) = <[u8; 3]>::try_from(mac) {
        return bacnet_transport::bip6::format_vmac_text(bacnet_transport::bip6::Bip6Vmac::new(
            vmac,
        ));
    }
    mac.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn decode_cov_value(data: &[u8]) -> String {
    let mut offset = 0;
    let mut values = Vec::new();
    while offset < data.len() {
        match decode_application_value(data, offset) {
            Ok((value, next)) => {
                values.push(crate::output::format_property_value(&value));
                offset = next;
            }
            Err(_) => {
                let hex: String = data[offset..]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                values.push(format!("[raw: {hex}]"));
                break;
            }
        }
    }
    values.join(", ")
}

#[cfg(test)]
mod tests {
    use super::{format_mac, renewal_interval};
    use std::time::Duration;

    /// Renewal comes 30 s before a subscription runs out, never before half
    /// of it, at least a second apart, and never for an indefinite one.
    #[test]
    fn finite_subscriptions_renew_before_they_run_out() {
        let secs = |s| Some(Duration::from_secs(s));
        assert_eq!(renewal_interval(300), secs(270));
        assert_eq!(renewal_interval(40), secs(20));
        assert_eq!(renewal_interval(1), secs(1));
        assert_eq!(renewal_interval(u32::MAX), secs(u64::from(u32::MAX) - 30));
        assert_eq!(renewal_interval(0), None);
    }
    use crate::core::target::{parse_target, Target};

    /// A COV source on B/IPv6 prints as the `bip6:` VMAC a target takes back;
    /// other MACs print as hex.
    #[test]
    fn cov_sources_print_b_ipv6_vmacs_as_targets() {
        let vmac = [0x00, 0x04, 0xD2];
        let printed = format_mac(&vmac);
        assert_eq!(printed, "bip6:0x0004D2");
        match parse_target(&printed) {
            Ok(Target::Mac(mac)) => assert_eq!(mac, vmac),
            _ => panic!("{printed} does not read back"),
        }
        assert_eq!(format_mac(&[2, 0, 0, 0, 0, 1]), "02:00:00:00:00:01");
        assert_eq!(format_mac(&[7]), "07");
    }
}
