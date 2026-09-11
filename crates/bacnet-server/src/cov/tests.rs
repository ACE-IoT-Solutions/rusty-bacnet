use super::*;
use bacnet_types::enums::ObjectType;
use std::time::Duration;

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

fn ai2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap()
}

fn make_sub(mac: &[u8], process_id: u32, oid: ObjectIdentifier) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(mac),
        subscriber_network: None,
        subscriber_process_identifier: process_id,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: false,
        expires_at: None,
        last_notified_value: None,
        monitored_property: None,
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    }
}

#[test]
fn subscribe_and_lookup() {
    let mut table = CovSubscriptionTable::new();
    table.subscribe(make_sub(&[1, 2, 3], 1, ai1()));
    assert_eq!(table.len(), 1);
    assert_eq!(table.subscriptions_for(&ai1()).len(), 1);
    assert_eq!(table.subscriptions_for(&ai2()).len(), 0);
}

#[test]
fn unsubscribe() {
    let mut table = CovSubscriptionTable::new();
    table.subscribe(make_sub(&[1, 2, 3], 1, ai1()));
    assert!(table.unsubscribe(&[1, 2, 3], 1, ai1()));
    assert!(!table.unsubscribe(&[1, 2, 3], 1, ai1())); // already removed
    assert!(table.is_empty());
}

#[test]
fn expired_subscriptions_purged_on_lookup() {
    let mut table = CovSubscriptionTable::new();
    let mut sub = make_sub(&[1, 2, 3], 1, ai1());
    sub.expires_at = Some(Instant::now() - Duration::from_secs(1)); // already expired
    table.subscribe(sub);
    assert_eq!(table.subscriptions_for(&ai1()).len(), 0);
    assert!(table.is_empty());
}

#[test]
fn multiple_subscribers_same_object() {
    let mut table = CovSubscriptionTable::new();
    table.subscribe(make_sub(&[1, 2, 3], 1, ai1()));
    table.subscribe(make_sub(&[4, 5, 6], 2, ai1()));
    assert_eq!(table.subscriptions_for(&ai1()).len(), 2);
}

fn snapshot(value: PropertyValue) -> CovValueSnapshot {
    CovValueSnapshot::new(&value, None).unwrap()
}

fn table_with_baseline(value: PropertyValue) -> (CovSubscriptionTable, CovSubscription) {
    let sub = make_sub(&[1, 2, 3], 1, ai1());
    let mut table = CovSubscriptionTable::new();
    table.subscribe(sub.clone());
    table.set_last_notified_snapshot(&[1, 2, 3], None, 1, ai1(), None, snapshot(value));
    (table, sub)
}

#[test]
fn should_notify_first_notification_always_fires() {
    let sub = make_sub(&[1, 2, 3], 1, ai1());
    let table = CovSubscriptionTable::new();
    let current = snapshot(PropertyValue::Real(72.5));
    assert!(table.should_notify(&sub, Some(&current), Some(1.0)));
}

#[test]
fn analog_snapshot_applies_increment_and_requires_a_change() {
    let (table, sub) = table_with_baseline(PropertyValue::Real(72.0));
    let below = snapshot(PropertyValue::Real(72.3));
    let exact = snapshot(PropertyValue::Real(73.0));
    let unchanged = snapshot(PropertyValue::Real(72.0));

    assert!(!table.should_notify(&sub, Some(&below), Some(1.0)));
    assert!(table.should_notify(&sub, Some(&exact), Some(1.0)));
    assert!(!table.should_notify(&sub, Some(&unchanged), Some(0.0)));
    assert!(table.should_notify(&sub, Some(&below), Some(0.0)));
}

#[test]
fn exact_snapshots_only_notify_on_binary_multistate_or_string_change() {
    for (baseline, unchanged, changed) in [
        (
            PropertyValue::Enumerated(0),
            PropertyValue::Enumerated(0),
            PropertyValue::Enumerated(1),
        ),
        (
            PropertyValue::Unsigned(2),
            PropertyValue::Unsigned(2),
            PropertyValue::Unsigned(3),
        ),
        (
            PropertyValue::CharacterString("idle".into()),
            PropertyValue::CharacterString("idle".into()),
            PropertyValue::CharacterString("active".into()),
        ),
    ] {
        let (table, sub) = table_with_baseline(baseline);
        let unchanged = snapshot(unchanged);
        let changed = snapshot(changed);
        assert!(!table.should_notify(&sub, Some(&unchanged), None));
        assert!(table.should_notify(&sub, Some(&changed), None));
    }
}

#[test]
fn whole_object_status_flags_change_bypasses_analog_increment() {
    let sub = make_sub(&[1, 2, 3], 1, ai1());
    let mut table = CovSubscriptionTable::new();
    table.subscribe(sub.clone());
    let baseline = CovValueSnapshot::new(
        &PropertyValue::Real(72.0),
        Some(&PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0],
        }),
    )
    .unwrap();
    table.set_last_notified_snapshot(&[1, 2, 3], None, 1, ai1(), None, baseline);
    let current = CovValueSnapshot::new(
        &PropertyValue::Real(72.1),
        Some(&PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0x80],
        }),
    )
    .unwrap();
    assert!(table.should_notify(&sub, Some(&current), Some(10.0)));
}

#[test]
fn oversized_exact_snapshot_is_not_retained() {
    let oversized = PropertyValue::OctetString(vec![0; MAX_COV_SNAPSHOT_COMPONENT_BYTES + 1]);
    assert!(CovValueSnapshot::new(&oversized, None).is_none());
}

#[test]
fn deterministic_thousand_update_fixture_applies_analog_and_exact_criteria() {
    fn notifications_for(
        initial: PropertyValue,
        updates: impl Iterator<Item = PropertyValue>,
        increment: Option<f32>,
    ) -> usize {
        let sub = make_sub(&[1, 2, 3], 1, ai1());
        let mut table = CovSubscriptionTable::new();
        table.subscribe(sub.clone());
        table.set_last_notified_snapshot(&[1, 2, 3], None, 1, ai1(), None, snapshot(initial));

        let mut notifications = 0;
        for value in updates {
            let current = snapshot(value);
            if table.should_notify(&sub, Some(&current), increment) {
                notifications += 1;
                table.set_last_notified_snapshot(&[1, 2, 3], None, 1, ai1(), None, current);
            }
        }
        notifications
    }

    assert_eq!(
        notifications_for(
            PropertyValue::Real(0.0),
            (1..=1_000).map(|value| PropertyValue::Real(value as f32)),
            Some(10.0),
        ),
        100
    );
    assert_eq!(
        notifications_for(
            PropertyValue::Enumerated(0),
            (1..=1_000).map(|value| PropertyValue::Enumerated(value % 2)),
            None,
        ),
        1_000
    );
    assert_eq!(
        notifications_for(
            PropertyValue::Unsigned(1),
            (1..=1_000).map(|value| PropertyValue::Unsigned((value % 3 + 1) as u64)),
            None,
        ),
        1_000
    );
    assert_eq!(
        notifications_for(
            PropertyValue::CharacterString("a".into()),
            (1..=1_000).map(|value| {
                PropertyValue::CharacterString(if value % 2 == 0 { "a" } else { "b" }.into())
            }),
            None,
        ),
        1_000
    );
}

#[test]
fn upsert_replaces_existing() {
    let mut table = CovSubscriptionTable::new();
    let mut sub = make_sub(&[1, 2, 3], 1, ai1());
    sub.issue_confirmed_notifications = false;
    table.subscribe(sub);
    // Same (mac, process_id, object) key — replaces the existing entry
    let mut sub2 = make_sub(&[1, 2, 3], 1, ai1());
    sub2.issue_confirmed_notifications = true;
    table.subscribe(sub2);
    assert_eq!(table.len(), 1);
    let subs = table.subscriptions_for(&ai1());
    assert!(subs[0].issue_confirmed_notifications);
}

#[test]
fn same_subscriber_different_objects_both_exist() {
    let mut table = CovSubscriptionTable::new();
    // Same (mac, process_id) but different monitored objects
    table.subscribe(make_sub(&[1, 2, 3], 1, ai1()));
    table.subscribe(make_sub(&[1, 2, 3], 1, ai2()));
    assert_eq!(table.len(), 2);
    assert_eq!(table.subscriptions_for(&ai1()).len(), 1);
    assert_eq!(table.subscriptions_for(&ai2()).len(), 1);
}

#[test]
fn purge_expired_removes_stale_subscriptions() {
    let mut table = CovSubscriptionTable::new();
    let mut sub1 = make_sub(&[1, 2, 3], 1, ai1());
    sub1.expires_at = Some(Instant::now() - Duration::from_secs(10));
    table.subscribe(sub1);

    let mut sub2 = make_sub(&[4, 5, 6], 2, ai1());
    sub2.expires_at = None; // infinite lifetime
    table.subscribe(sub2);

    let purged = table.purge_expired();
    assert_eq!(purged, 1);
    assert_eq!(table.len(), 1);
}

#[test]
fn cov_multiple_context_lifetime_refreshes_and_expires() {
    let mut table = CovSubscriptionTable::new();
    let original_expiry = Instant::now() + Duration::from_secs(30);
    let refreshed_expiry = Instant::now() + Duration::from_secs(60);

    let mut present_value = make_sub(&[1, 2, 3], 1, ai1());
    present_value.notification_kind = CovNotificationKind::Multiple;
    present_value.monitored_property = Some(PropertyIdentifier::PRESENT_VALUE);
    present_value.expires_at = Some(original_expiry);
    table.subscribe(present_value);

    let mut status_flags = make_sub(&[1, 2, 3], 1, ai1());
    status_flags.notification_kind = CovNotificationKind::Multiple;
    status_flags.monitored_property = Some(PropertyIdentifier::STATUS_FLAGS);
    status_flags.expires_at = Some(original_expiry);
    table.subscribe(status_flags);

    let mut single = make_sub(&[1, 2, 3], 1, ai1());
    single.expires_at = Some(original_expiry);
    table.subscribe(single);

    table.refresh_cov_multiple_context_lifetime(&[1, 2, 3], None, 1, false, Some(refreshed_expiry));

    let multiple_expiries: Vec<_> = table
        .subs
        .values()
        .filter(|sub| sub.notification_kind == CovNotificationKind::Multiple)
        .map(|sub| sub.expires_at)
        .collect();
    assert_eq!(
        multiple_expiries,
        vec![Some(refreshed_expiry), Some(refreshed_expiry)]
    );
    assert!(table
        .subs
        .values()
        .any(|sub| sub.notification_kind == CovNotificationKind::Single
            && sub.expires_at == Some(original_expiry)));

    table.refresh_cov_multiple_context_lifetime(
        &[1, 2, 3],
        None,
        1,
        false,
        Some(Instant::now() - Duration::from_secs(1)),
    );

    assert_eq!(table.purge_expired(), 2);
    assert_eq!(table.len(), 1);
    assert!(table.subs.values().all(|sub| {
        sub.notification_kind == CovNotificationKind::Single
            && sub.expires_at == Some(original_expiry)
    }));
}

#[test]
fn purge_expired_returns_zero_when_none_expired() {
    let mut table = CovSubscriptionTable::new();
    table.subscribe(make_sub(&[1, 2, 3], 1, ai1()));
    let purged = table.purge_expired();
    assert_eq!(purged, 0);
    assert_eq!(table.len(), 1);
}

#[test]
fn default_policy_allows_1024th_subscription() {
    let mut table = CovSubscriptionTable::new();
    let oid = ai1();
    // 16 peers * 64 subscriptions each = 1024 subscriptions
    for peer_idx in 0..16u8 {
        let mac = [192, 168, 1, peer_idx];
        let peer_key = CovPeerKey::direct(MacAddr::from_slice(&mac));
        for proc_id in 0..64u32 {
            table
                .check_admission(&peer_key, false, None)
                .expect("subscription admitted");
            let mut sub = make_sub(&mac, proc_id, oid);
            sub.expires_at = Some(Instant::now() + Duration::from_secs(300));
            table.subscribe(sub);
        }
    }
    assert_eq!(table.len(), 1024);

    // 1025th subscription from a 17th peer fails due to global capacity
    let mac17 = [192, 168, 1, 17];
    let peer17 = CovPeerKey::direct(MacAddr::from_slice(&mac17));
    assert!(table.check_admission(&peer17, false, None).is_err());
}

#[test]
fn effective_unreserved_capacity_respects_reserved_peers() {
    let mut policy = CovPolicy {
        max_subscriptions_global: 100,
        reserved_capacity: 20,
        reserved_peers: Vec::new(),
        ..Default::default()
    };
    // No reserved peers -> full global capacity available
    assert_eq!(policy.effective_unreserved_capacity(), 100);

    // With reserved peers -> reserved capacity is deducted
    policy.reserved_peers.push(MacAddr::from_slice(&[1, 2, 3]));
    assert_eq!(policy.effective_unreserved_capacity(), 80);

    // If reserved_capacity is 0 -> full global capacity
    policy.reserved_capacity = 0;
    assert_eq!(policy.effective_unreserved_capacity(), 100);
}

#[test]
fn in_flight_tracker_does_not_leak_zero_count_entries_on_failure() {
    let tracker = Arc::new(CovInFlightTracker::default());
    let semaphore = Arc::new(tokio::sync::Semaphore::new(0));
    let peer = CovPeerKey::direct(MacAddr::from_slice(&[1, 2, 3, 4]));

    // Acquisition fails due to global pool exhausted
    let err = tracker
        .try_acquire(peer.clone(), 10, &semaphore)
        .unwrap_err();
    assert_eq!(err, InFlightAcquireError::GlobalPoolExhausted);
    assert_eq!(tracker.active_peer_count(), 0);

    // Acquisition fails due to peer limit exceeded (max_per_peer = 0)
    let semaphore2 = Arc::new(tokio::sync::Semaphore::new(10));
    let peer2 = CovPeerKey::direct(MacAddr::from_slice(&[5, 6, 7, 8]));
    let err2 = tracker.try_acquire(peer2, 0, &semaphore2).unwrap_err();
    assert_eq!(err2, InFlightAcquireError::PeerLimitExceeded);
    assert_eq!(tracker.active_peer_count(), 0);
}

#[test]
fn expired_subscriptions_immediately_release_quota_on_admission() {
    let policy = CovPolicy {
        max_subscriptions_per_peer: 1,
        ..Default::default()
    };
    let mut table =
        CovSubscriptionTable::with_policy(policy, Arc::new(AtomicCovCounters::default()));
    let peer = CovPeerKey::direct(MacAddr::from_slice(&[1, 2, 3]));

    // Create a subscription with an expiry in the past
    let mut sub = make_sub(&[1, 2, 3], 1, ai1());
    sub.expires_at = Some(Instant::now() - Duration::from_secs(5));
    table.subscribe(sub);
    assert_eq!(table.len(), 1);

    // Admitting a new subscription from the same peer immediately purges the expired subscription
    // and succeeds, rather than being rejected by per-peer quota!
    assert!(table.check_admission(&peer, false, None).is_ok());
    assert_eq!(table.len(), 0);
}

#[test]
fn is_peer_reserved_checks_canonical_peer_identity() {
    let direct_mac = MacAddr::from_slice(&[1, 2, 3]);
    let policy = CovPolicy {
        reserved_peers: vec![direct_mac.clone()],
        reserved_peer_keys: vec![CovPeerKey::routed(10, direct_mac.clone())],
        ..Default::default()
    };

    // Direct peer matching reserved_peers is reserved
    assert!(policy.is_peer_reserved(&CovPeerKey::direct(direct_mac.clone())));

    // Routed peer on network 10 matching reserved_peer_keys is reserved
    assert!(policy.is_peer_reserved(&CovPeerKey::routed(10, direct_mac.clone())));

    // Routed peer on different network (20) with same MAC is NOT reserved
    assert!(!policy.is_peer_reserved(&CovPeerKey::routed(20, direct_mac.clone())));

    // Unconfigured direct peer is NOT reserved
    let other_mac = MacAddr::from_slice(&[4, 5, 6]);
    assert!(!policy.is_peer_reserved(&CovPeerKey::direct(other_mac.clone())));
    assert!(!policy.is_peer_reserved(&CovPeerKey::routed(10, other_mac)));
}
