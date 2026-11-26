//! Admission at completion and fair expiry under load.
use super::*;

#[tokio::test(start_paused = true)]
async fn a_due_transfer_expires_before_a_busy_peers_queued_segments() {
    use bacnet_network::response_route::ResponseRoute;

    let mut lab = lab(true).await;
    let whole = cov_body(7);
    let Apdu::ConfirmedRequest(first) = segment(1, COV, 0, 1, true, whole.slice(..4)) else {
        unreachable!()
    };
    let route = ResponseRoute::unverified();
    // The peer is silent long enough to hit the no-progress timer while
    // its segment timeout is still in the future.
    let mut table = SegmentedRequests::new(Duration::from_secs(30));
    assert!(matches!(
        table.receive(
            Segment {
                source_mac: &PEER,
                source_network: None,
                route: &route,
                request: &first
            },
            now()
        ),
        Receipt::Ack(_)
    ));
    tokio::time::advance(bacnet_endpoint_core::segmented_request::PROGRESS_TIMEOUT).await;

    let (queue, receiver) = mpsc::channel(16);
    for id in 0..16 {
        let mut request = first.clone();
        request.invoke_id = id;
        queue
            .try_send(Handed::Segment {
                source_mac: MacAddr::from_slice(&OTHER),
                source_network: None,
                reply: InboundReply::new(route.clone(), None),
                request,
            })
            .unwrap();
    }
    let worker = tokio::spawn(BACnetClient::reassemble(
        Arc::downgrade(&lab.client.network),
        receiver,
        Arc::new(SyncMutex::new(table)),
        Outlets {
            _hold: None,
            event_tx: lab.client.event_tx.clone(),
            cov_tx: lab.client.cov_tx.clone(),
            policy: cov_notifications::default_confirmed_cov_notification_ack_policy(),
            registry: None,
            decode_warning: WarnThrottle::default(),
            response_warning: WarnThrottle::default(),
        },
    ));
    // Both sources are ready on the very first worker poll.
    aborted(lab.next(&PEER).await, 1, AbortReason::OTHER);
    segment_ack(lab.next(&OTHER).await, 0, 0, false, 1);
    drop(queue);
    worker.await.unwrap();
    lab.client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn strict_worker_declines_multiple_and_checks_the_current_lease_at_completion() {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    let mut lab = lab_with(
        true,
        None,
        ClientOptions {
            confirmed_cov_subscription_limit: std::num::NonZeroUsize::new(1),
            ..ClientOptions::default()
        },
    )
    .await;
    let process = 11;
    let n = cov_notification(process);
    let lease = lab
        .client
        .expect_confirmed_cov(
            Station::local(&PEER),
            process,
            n.monitored_object_identifier,
            None,
        )
        .unwrap();
    let mut events = lab.client.cov_notifications();
    let (_, multiple) = multiple_body(process, 3);
    for (id, service, body) in [(10, COV_MULTIPLE, multiple), (11, COV, cov_body(process))] {
        let cut = body.len() / 2;
        lab.from(&PEER, &segment(id, service, 0, 2, true, body.slice(..cut)))
            .await;
        segment_ack(lab.next(&PEER).await, id, 0, false, 2);
        if service == COV {
            lease.close();
        }
        lab.from(&PEER, &segment(id, service, 1, 2, false, body.slice(cut..)))
            .await;
        segment_ack(lab.next(&PEER).await, id, 1, false, 2);
        let Apdu::Error(failure) = lab.next(&PEER).await else {
            panic!("expected an Error")
        };
        assert_eq!((failure.invoke_id, failure.service_choice), (id, service));
        assert_eq!(
            (failure.error_class, failure.error_code),
            (ErrorClass::SERVICES, ErrorCode::UNKNOWN_SUBSCRIPTION)
        );
    }
    assert!(events.try_recv().is_err());
    lab.client.stop().await.unwrap();
}

/// A node closing the seat fences the worker but leaves its join available
/// to a client stop racing the node's remaining cleanup.
#[tokio::test(start_paused = true)]
async fn coordinated_close_retains_the_blocked_workers_join_for_client_stop() {
    let mut lab = lab(true).await;
    let handle = lab.client.transport().handle();
    handle.block_next_send();
    lab.from(&PEER, &segments(17, COV, &cov_body(52), 8, 1)[0])
        .await;
    handle.wait_blocked().await;
    let table = lab.table().unwrap();
    {
        let mut tsm = lab.client.tsm.lock();
        tsm.close_coordinated();
        assert!(tsm.request_reassembly.as_ref().unwrap().task.is_some());
    }
    lab.client.stop().await.unwrap();
    assert!(table.upgrade().is_none());
    assert!(lab.table().is_none());
}
