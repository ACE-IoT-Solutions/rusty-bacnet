use super::*;

fn exercise<Q: QueueStorage>(mut queue: Q) {
    assert_eq!(
        queue.enqueue_npdu(1, &[1], NetworkPriority::Normal, Instant::ZERO, None),
        Ok(())
    );
    assert_eq!(
        queue.enqueue_npdu(2, &[2], NetworkPriority::LifeSafety, Instant::ZERO, None),
        Ok(())
    );
    assert_eq!(
        queue.enqueue_npdu(
            QueueKind::BROADCAST_DESTINATION,
            &[3],
            NetworkPriority::Normal,
            Instant::ZERO,
            None
        ),
        Ok(())
    );
    let first = queue.dequeue(Instant::ZERO, 4).unwrap().unwrap();
    assert_eq!((first.destination, first.npdu), (2, &[2][..]));
    let second = queue.dequeue(Instant::ZERO, 4).unwrap().unwrap();
    assert_eq!((second.destination, second.npdu), (1, &[1][..]));
    let third = queue.dequeue(Instant::ZERO, 4).unwrap().unwrap();
    assert_eq!((third.kind, third.npdu), (QueueKind::Broadcast, &[3][..]));
}

#[test]
fn priority_and_destination_selection() {
    #[cfg(feature = "heapless")]
    exercise(HeaplessQueues::<4, 4>::new());
    #[cfg(feature = "alloc")]
    exercise(AllocQueues::<MAX_STANDARD_MPDU_DATA>::new(4, 4));
}

/// An item longer than the caller takes is dropped and counted, so it can't
/// stay at the head of its queue.
fn drop_item_too_long_for_scratch<Q: QueueStorage>(mut queue: Q) {
    queue
        .enqueue_npdu(1, &[1, 2, 3], NetworkPriority::Urgent, Instant::ZERO, None)
        .unwrap();
    queue
        .enqueue_npdu(1, &[4], NetworkPriority::Normal, Instant::ZERO, None)
        .unwrap();
    assert_eq!(
        queue.dequeue(Instant::ZERO, 2),
        Err(QueueRejectReason::Oversize)
    );
    let stats = queue.stats(QueueKind::Unicast);
    assert_eq!((stats.depth, stats.drops_oversize), (1, 1));
    let next = queue.dequeue(Instant::ZERO, 2).unwrap().unwrap();
    assert_eq!(next.npdu, [4]);
}

#[test]
fn an_item_too_long_for_the_scratch_is_dropped() {
    #[cfg(feature = "heapless")]
    drop_item_too_long_for_scratch(HeaplessQueues::<4, 4>::new());
    #[cfg(feature = "alloc")]
    drop_item_too_long_for_scratch(AllocQueues::<MAX_STANDARD_MPDU_DATA>::new(4, 4));
}

#[test]
fn exact_expiry_boundary_and_atomic_oversize() {
    #[cfg(feature = "heapless")]
    {
        let mut queue = HeaplessQueues::<1, 1>::new();
        assert_eq!(
            queue.enqueue_npdu(
                1,
                &[9],
                NetworkPriority::Normal,
                Instant::from_nanos(4),
                Some(Instant::from_nanos(4))
            ),
            Err(QueueRejectReason::Expired)
        );
        assert_eq!(queue.depth(QueueKind::Unicast), 0);
        assert_eq!(
            queue.enqueue_npdu(1, &[1, 2], NetworkPriority::Normal, Instant::ZERO, None),
            Ok(())
        );
        assert_eq!(
            queue.enqueue_npdu(1, &[3], NetworkPriority::Normal, Instant::ZERO, None),
            Err(QueueRejectReason::Full)
        );
        assert_eq!(queue.depth(QueueKind::Unicast), 1);
    }
    #[cfg(feature = "alloc")]
    {
        let mut queue = AllocQueues::<MAX_STANDARD_MPDU_DATA>::new(1, 1);
        assert_eq!(
            queue.enqueue_npdu(
                1,
                &[9],
                NetworkPriority::Normal,
                Instant::from_nanos(4),
                Some(Instant::from_nanos(4))
            ),
            Err(QueueRejectReason::Expired)
        );
        assert_eq!(queue.depth(QueueKind::Unicast), 0);
        assert_eq!(
            queue.enqueue_npdu(1, &[1, 2], NetworkPriority::Normal, Instant::ZERO, None),
            Ok(())
        );
        assert_eq!(
            queue.enqueue_npdu(1, &[3], NetworkPriority::Normal, Instant::ZERO, None),
            Err(QueueRejectReason::Full)
        );
        assert_eq!(queue.depth(QueueKind::Unicast), 1);
    }
}

#[test]
fn starvation_forces_old_item_after_service_opportunities() {
    #[cfg(feature = "heapless")]
    {
        let mut queue = HeaplessQueues::<4, 0>::with_starvation_limit(1);
        queue
            .enqueue_npdu(1, &[1], NetworkPriority::Normal, Instant::ZERO, None)
            .unwrap();
        queue
            .enqueue_npdu(1, &[2], NetworkPriority::LifeSafety, Instant::ZERO, None)
            .unwrap();
        assert_eq!(queue.dequeue(Instant::ZERO, 2).unwrap().unwrap().npdu, [2]);
        assert_eq!(queue.dequeue(Instant::ZERO, 2).unwrap().unwrap().npdu, [1]);
    }
    #[cfg(feature = "alloc")]
    {
        let mut queue = AllocQueues::<MAX_STANDARD_MPDU_DATA>::with_starvation_limit(4, 0, 1);
        queue
            .enqueue_npdu(1, &[1], NetworkPriority::Normal, Instant::ZERO, None)
            .unwrap();
        queue
            .enqueue_npdu(1, &[2], NetworkPriority::LifeSafety, Instant::ZERO, None)
            .unwrap();
        assert_eq!(queue.dequeue(Instant::ZERO, 2).unwrap().unwrap().npdu, [2]);
        assert_eq!(queue.dequeue(Instant::ZERO, 2).unwrap().unwrap().npdu, [1]);
    }
}

#[test]
fn kind_specific_selection_preserves_independent_capacity() {
    #[cfg(feature = "heapless")]
    {
        let mut queue = HeaplessQueues::<2, 1>::new();
        queue
            .enqueue_npdu(1, &[1], NetworkPriority::Normal, Instant::ZERO, None)
            .unwrap();
        queue
            .enqueue_npdu(0xff, &[2], NetworkPriority::Normal, Instant::ZERO, None)
            .unwrap();
        assert_eq!(
            queue
                .peek_kind(QueueKind::Broadcast, Instant::ZERO)
                .unwrap()
                .kind,
            QueueKind::Broadcast
        );
        assert_eq!(
            queue
                .peek_kind(QueueKind::Unicast, Instant::ZERO)
                .unwrap()
                .kind,
            QueueKind::Unicast
        );
    }
    #[cfg(feature = "alloc")]
    {
        let mut queue = AllocQueues::<MAX_STANDARD_MPDU_DATA>::new(2, 1);
        queue
            .enqueue_npdu(1, &[1], NetworkPriority::Normal, Instant::ZERO, None)
            .unwrap();
        queue
            .enqueue_npdu(0xff, &[2], NetworkPriority::Normal, Instant::ZERO, None)
            .unwrap();
        assert_eq!(
            queue
                .peek_kind(QueueKind::Broadcast, Instant::ZERO)
                .unwrap()
                .kind,
            QueueKind::Broadcast
        );
        assert_eq!(
            queue
                .peek_kind(QueueKind::Unicast, Instant::ZERO)
                .unwrap()
                .kind,
            QueueKind::Unicast
        );
    }
}

/// Dequeue every item, up to four, and return each one's payload octet in
/// the order they leave.
fn drain_first_octets<Q: QueueStorage>(queue: &mut Q, now: Instant) -> [u8; 4] {
    let mut order = [0; 4];
    for octet in &mut order {
        let item = queue.dequeue(now, 4).unwrap().unwrap();
        assert_eq!(item.npdu.len(), 1);
        *octet = item.npdu[0];
    }
    assert_eq!(queue.dequeue(now, 4), Ok(None));
    order
}

/// Items leave by priority, then oldest first, wherever they lie: an item
/// written into a slot freed in the middle, or before older items, still
/// leaves after every older item of its priority.
fn freed_slots_keep_the_order<Q: QueueStorage>(mut queue: Q) {
    let now = Instant::ZERO;
    for (octet, priority) in [
        (1, NetworkPriority::Normal),
        (2, NetworkPriority::Urgent),
        (3, NetworkPriority::Normal),
        (4, NetworkPriority::Normal),
    ] {
        queue
            .enqueue_npdu(1, &[octet], priority, now, None)
            .unwrap();
    }
    // The urgent item leaves first, freeing the second slot.
    assert_eq!(queue.dequeue(now, 4).unwrap().unwrap().npdu, [2]);
    // The newest item takes that slot, ahead of two older ones; a fifth
    // finds the queue full.
    queue
        .enqueue_npdu(1, &[5], NetworkPriority::Normal, now, None)
        .unwrap();
    assert_eq!(
        queue.enqueue_npdu(1, &[6], NetworkPriority::Normal, now, None),
        Err(QueueRejectReason::Full)
    );
    assert_eq!(queue.depth(QueueKind::Unicast), 4);
    assert_eq!(drain_first_octets(&mut queue, now), [1, 3, 4, 5]);
    let stats = queue.stats(QueueKind::Unicast);
    assert_eq!(
        (
            stats.depth,
            stats.high_water,
            stats.enqueued,
            stats.dequeued
        ),
        (0, 4, 5, 5)
    );
    assert_eq!(stats.drops_full, 1);

    // Expiry frees slots in the middle too, and new items reuse them.
    let at = Instant::from_nanos;
    queue
        .enqueue_npdu(1, &[7], NetworkPriority::Normal, now, None)
        .unwrap();
    queue
        .enqueue_npdu(1, &[8], NetworkPriority::Normal, now, Some(at(5)))
        .unwrap();
    queue
        .enqueue_npdu(1, &[9], NetworkPriority::Normal, now, None)
        .unwrap();
    assert_eq!(queue.expire(at(5)), 1);
    queue
        .enqueue_npdu(1, &[10], NetworkPriority::Normal, at(5), None)
        .unwrap();
    queue
        .enqueue_npdu(1, &[11], NetworkPriority::Normal, at(5), None)
        .unwrap();
    assert_eq!(drain_first_octets(&mut queue, at(5)), [7, 9, 10, 11]);
    let stats = queue.stats(QueueKind::Unicast);
    assert_eq!((stats.depth, stats.drops_expired), (0, 1));
}

#[test]
fn freed_slots_are_reused_without_changing_the_order() {
    #[cfg(feature = "heapless")]
    freed_slots_keep_the_order(HeaplessQueues::<4, 1>::new());
    #[cfg(feature = "alloc")]
    freed_slots_keep_the_order(AllocQueues::<MAX_STANDARD_MPDU_DATA>::new(4, 1));
}

/// An item that has waited the starvation limit's number of service
/// opportunities goes first, ahead of a higher priority, and the oldest
/// starved item first.
fn starved_items_go_first<Q: QueueStorage>(mut queue: Q) {
    queue.set_starvation_limit(2);
    let now = Instant::ZERO;
    for (octet, priority) in [
        (1, NetworkPriority::Normal),
        (2, NetworkPriority::Normal),
        (3, NetworkPriority::LifeSafety),
        (4, NetworkPriority::LifeSafety),
    ] {
        queue
            .enqueue_npdu(1, &[octet], priority, now, None)
            .unwrap();
    }
    // Two service opportunities for the normal items: both now starve.
    assert_eq!(queue.dequeue(now, 4).unwrap().unwrap().npdu, [3]);
    assert_eq!(queue.dequeue(now, 4).unwrap().unwrap().npdu, [4]);
    queue
        .enqueue_npdu(1, &[5], NetworkPriority::LifeSafety, now, None)
        .unwrap();
    assert_eq!(queue.peek(now).unwrap().service_wait, 2);
    for octet in [1, 2, 5] {
        assert_eq!(queue.dequeue(now, 4).unwrap().unwrap().npdu, [octet]);
    }
    assert_eq!(queue.dequeue(now, 4), Ok(None));
}

#[test]
fn starved_items_go_ahead_of_higher_priorities() {
    #[cfg(feature = "heapless")]
    starved_items_go_first(HeaplessQueues::<4, 1>::new());
    #[cfg(feature = "alloc")]
    starved_items_go_first(AllocQueues::<MAX_STANDARD_MPDU_DATA>::new(4, 1));
}

/// A peek never shows an expired item, and changes nothing.
fn peeks_skip_expired_items<Q: QueueStorage>(mut queue: Q) {
    let at = Instant::from_nanos;
    let now = Instant::ZERO;
    queue
        .enqueue_npdu(1, &[1], NetworkPriority::Urgent, now, Some(at(5)))
        .unwrap();
    queue
        .enqueue_npdu(1, &[2], NetworkPriority::Normal, now, None)
        .unwrap();
    queue
        .enqueue_npdu(0xff, &[3], NetworkPriority::Normal, now, Some(at(5)))
        .unwrap();
    let urgent = queue.peek(at(4)).unwrap();
    assert_eq!((urgent.priority, urgent.len), (NetworkPriority::Urgent, 1));
    assert!(queue.peek_kind(QueueKind::Broadcast, at(4)).is_some());
    assert_eq!(queue.peek(at(5)).unwrap().priority, NetworkPriority::Normal);
    assert_eq!(queue.peek_kind(QueueKind::Broadcast, at(5)), None);
    assert_eq!(
        (
            queue.depth(QueueKind::Unicast),
            queue.depth(QueueKind::Broadcast)
        ),
        (2, 1)
    );
}

#[test]
fn peeks_never_show_an_expired_item() {
    #[cfg(feature = "heapless")]
    peeks_skip_expired_items(HeaplessQueues::<2, 1>::new());
    #[cfg(feature = "alloc")]
    peeks_skip_expired_items(AllocQueues::<MAX_STANDARD_MPDU_DATA>::new(2, 1));
}

/// A slot takes an NPDU as long as itself, and refuses a longer one,
/// counted, on either queue.
fn npdus_longer_than_a_slot_are_refused<Q: QueueStorage>(mut queue: Q) {
    let now = Instant::ZERO;
    let long = [0x5A; MAX_STANDARD_MPDU_DATA + 1];
    for destination in [1, 0xff] {
        assert_eq!(
            queue.enqueue_npdu(destination, &long, NetworkPriority::Normal, now, None),
            Err(QueueRejectReason::Oversize)
        );
    }
    for kind in [QueueKind::Unicast, QueueKind::Broadcast] {
        let stats = queue.stats(kind);
        assert_eq!(
            (stats.depth, stats.enqueued, stats.drops_oversize),
            (0, 0, 1)
        );
    }
    let whole = &long[..MAX_STANDARD_MPDU_DATA];
    queue
        .enqueue_npdu(1, whole, NetworkPriority::Normal, now, None)
        .unwrap();
    let item = queue.dequeue(now, MAX_STANDARD_MPDU_DATA).unwrap().unwrap();
    assert_eq!(item.npdu, whole);
}

#[test]
fn a_slot_holds_its_length_and_no_more() {
    #[cfg(feature = "heapless")]
    npdus_longer_than_a_slot_are_refused(HeaplessQueues::<1, 1>::new());
    #[cfg(feature = "alloc")]
    npdus_longer_than_a_slot_are_refused(AllocQueues::<MAX_STANDARD_MPDU_DATA>::new(1, 1));
}
