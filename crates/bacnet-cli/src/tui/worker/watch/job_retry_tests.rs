//! Typed subscription failures and route exclusions through real client jobs.

use bacnet_client::discovery::RoutedDeviceConfig;
use bacnet_types::enums::{AbortReason, ObjectType, RejectReason, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;

use super::super::retry_support::{self, Reply};
use super::*;

fn point() -> Point {
    Point {
        device: 100,
        network: None,
        object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        property: PropertyIdentifier::PRESENT_VALUE,
    }
}

#[tokio::test(start_paused = true)]
async fn subscription_retry_classification_uses_typed_failures() {
    for (reply, retryable) in [
        (Reply::Silent, true),
        (Reply::Transport, true),
        (Reply::Abort(AbortReason::TSM_TIMEOUT.to_raw()), true),
        (Reply::Abort(AbortReason::OTHER.to_raw()), false),
        (
            Reply::Reject(RejectReason::UNRECOGNIZED_SERVICE.to_raw()),
            false,
        ),
        (Reply::Protocol, false),
    ] {
        let (client, _) = retry_support::client(reply).await;
        let result = subscribe(
            &client,
            &RequestLimits::new(0),
            &CovTarget::local(&[2]),
            1,
            point(),
            Lifetime::new(300, Duration::from_secs(8)),
        )
        .await;
        let Err(refusal) = result else {
            panic!("expected refusal")
        };
        assert_eq!(refusal.retryable, retryable, "{refusal:?}");
        assert_eq!(refusal.maybe_subscribed, retryable, "{refusal:?}");
        assert!(!refusal.fatal);
        retry_support::stop(client).await;
    }
}

/// A routed row subscribes through its router (#1790): a first read the
/// device never answers is retried as a local one's is, and any other
/// failure goes on to the subscription, which fails here too.
#[tokio::test(start_paused = true)]
async fn routed_rows_subscribe_through_their_router_and_retry_like_local_ones() {
    for reply in [Reply::Silent, Reply::Transport, Reply::SendProtocol] {
        let (client, _) = retry_support::client(reply).await;
        client
            .add_routed_device(RoutedDeviceConfig {
                instance: 100,
                router_mac: vec![2],
                remote_network: 10,
                remote_mac: vec![3],
                max_apdu_length: 1476,
                segmentation_supported: Segmentation::NONE,
                max_segments_accepted: None,
            })
            .await
            .unwrap();
        let result = start(
            &client,
            &RequestLimits::new(0),
            1,
            point(),
            Lifetime::new(300, Duration::from_secs(8)),
            true,
        )
        .await;
        assert_eq!(result.target, Some(CovTarget::routed(&[2], 10, &[3])));
        let Err(refusal) = result.outcome else {
            panic!("the peer never answers, yet the row subscribed")
        };
        assert!(!refusal.maybe_subscribed && !refusal.fatal, "{refusal:?}");
        if matches!(reply, Reply::SendProtocol) {
            assert!(!refusal.retryable, "{refusal:?}");
            assert!(refusal.reason.starts_with("COV refused:"), "{refusal:?}");
        } else {
            assert!(refusal.retryable, "{refusal:?}");
            assert!(refusal.reason.starts_with("no answer:"), "{refusal:?}");
        }
        retry_support::stop(client).await;
    }
}

#[tokio::test(start_paused = true)]
async fn missing_device_is_fatal_without_retry_or_cancellation() {
    let (client, requests) = retry_support::client(Reply::Silent).await;
    let result = start(
        &client,
        &RequestLimits::new(0),
        1,
        point(),
        Lifetime::new(300, Duration::from_secs(8)),
        true,
    )
    .await;
    assert!(result.target.is_none());
    let Err(refusal) = result.outcome else {
        panic!("missing device subscribed")
    };
    assert!(refusal.fatal && !refusal.retryable && !refusal.maybe_subscribed);
    assert_eq!(requests.load(std::sync::atomic::Ordering::Relaxed), 0);
    retry_support::stop(client).await;
}
