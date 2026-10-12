use super::*;

const PERIOD: Duration = Duration::from_millis(400);

async fn peer_and_transport() -> (UdpSocket, BipTransport) {
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: peer.local_addr().unwrap().port(),
        ttl: 2,
        renewal_interval: Some(PERIOD),
    });
    (peer, transport)
}

async fn registration(peer: &UdpSocket) -> std::net::SocketAddr {
    let mut packet = [0; 64];
    let (len, source) = tokio::time::timeout(Duration::from_secs(2), peer.recv_from(&mut packet))
        .await
        .expect("scheduled registration did not arrive")
        .unwrap();
    assert_eq!(&packet[..len], &[0x81, 5, 0, 6, 0, 2]);
    source
}

async fn observed(
    transport: &BipTransport,
    predicate: impl Fn(BvlcClientSnapshot) -> bool,
) -> BvlcClientSnapshot {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let snapshot = transport.bvlc_client_snapshot();
            if predicate(snapshot) {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("management observation did not arrive")
}

#[tokio::test]
async fn foreign_mode_validation_precedes_interface_or_socket_io() {
    for (ttl, interval) in [
        (0, None),
        (0, Some(Duration::from_millis(1))),
        (1, Some(Duration::ZERO)),
        (1, Some(Duration::from_secs(1))),
        (1, Some(Duration::from_secs(2))),
    ] {
        // A nonlocal interface would yield a transport error if validation ran late.
        let mut transport = BipTransport::new(Ipv4Addr::new(192, 0, 2, 1), 0, Ipv4Addr::BROADCAST);
        transport.register_as_foreign_device(ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::LOCALHOST,
            bbmd_port: 47808,
            ttl,
            renewal_interval: interval,
        });
        let error = transport.start().await.unwrap_err();
        assert!(matches!(error, Error::Encoding(_)), "{error:?}");
        assert!(error.to_string().contains("renewal interval"));
        assert!(transport.socket.is_none());
        assert!(!transport.bvlc_client_snapshot().running);
        assert_eq!(
            transport
                .bvlc_client_snapshot()
                .foreign_registration
                .attempts,
            0
        );
    }
}

#[tokio::test]
async fn foreign_attempt_results_renewal_and_restart_are_observable() {
    let (peer, mut transport) = peer_and_transport().await;
    let _rx = transport.start().await.unwrap();
    assert!(transport.bvlc_client_snapshot().running);
    let source = registration(&peer).await;
    let pending = transport.bvlc_client_snapshot();
    assert_eq!(
        (
            pending.foreign_registration.attempts,
            pending.foreign_registration.accepted
        ),
        (1, 0)
    );
    assert_eq!(pending.foreign_registration.last_ttl, Some(2));
    assert_eq!(
        pending.foreign_registration.last_outcome,
        Some(ForeignRegistrationOutcome::Pending)
    );
    assert!(pending.foreign_registration.next_attempt_in.unwrap() <= PERIOD);
    peer.send_to(&[0x81, 0, 0, 6, 0, 0x30], source)
        .await
        .unwrap();
    let refused = observed(&transport, |s| s.foreign_registration.refused == 1).await;
    assert_eq!(
        refused.foreign_registration.last_result,
        Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK)
    );
    let renewed = tokio::time::Instant::now();
    assert_eq!(registration(&peer).await, source);
    assert!(renewed.elapsed() < Duration::from_secs(1));
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    let accepted = observed(&transport, |s| s.foreign_registration.accepted == 1).await;
    assert_eq!(
        (
            accepted.register_foreign_device.sent,
            accepted.register_foreign_device.results
        ),
        (2, 2)
    );
    assert_eq!(
        accepted.foreign_registration.last_result,
        Some(BvlcResultCode::SUCCESSFUL_COMPLETION)
    );
    let worker = transport.node_task.as_ref().unwrap().abort_handle();
    transport.stop().await.unwrap();
    assert!(worker.is_finished());
    let stopped = transport.bvlc_client_snapshot();
    assert!(!stopped.running);
    assert_eq!(stopped.foreign_registration.last_outcome, None);
    assert_eq!(stopped.foreign_registration.last_ttl, None);
    assert_eq!(stopped.foreign_registration.last_result, None);
    assert_eq!(stopped.register_foreign_device.last_result, None);
    assert_eq!(stopped.foreign_registration.next_attempt_in, None);
    assert_eq!(stopped.foreign_registration.accepted, 1);
    let _rx = transport.start().await.unwrap();
    let restarted = transport.bvlc_client_snapshot();
    assert_eq!(restarted.foreign_registration.last_outcome, None);
    assert_eq!(registration(&peer).await, source);
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    observed(&transport, |s| s.foreign_registration.accepted == 2).await;
    transport.stop().await.unwrap();
}

/// A foreign device manages its own registration: it refuses the requests
/// a success answers (Register, Delete-FDT-Entry, Write-BDT) to its BBMD,
/// without sending them, and takes the others, and any to another peer.
#[tokio::test]
async fn foreign_mode_refuses_manual_requests_that_could_change_its_registration() {
    let (peer, mut transport) = peer_and_transport().await;
    let _rx = transport.start().await.unwrap();
    let source = registration(&peer).await;
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    observed(&transport, |s| s.foreign_registration.accepted == 1).await;
    let bbmd = encode_bip_mac(
        Ipv4Addr::LOCALHOST.octets(),
        peer.local_addr().unwrap().port(),
    );
    let refusals = [
        transport.register_foreign_device_bvlc(&bbmd, 60).await,
        transport
            .delete_fdt_entry(&bbmd, [10, 0, 0, 2], 47808)
            .await,
        transport.write_bdt(&bbmd, &[]).await,
    ];
    for refusal in refusals {
        let error = refusal.unwrap_err();
        assert!(matches!(error, Error::Encoding(_)), "{error:?}");
        assert!(
            error.to_string().contains("managed automatically"),
            "{error}"
        );
    }
    let snapshot = transport.bvlc_client_snapshot();
    assert_eq!(snapshot.write_bdt.sent + snapshot.delete_fdt_entry.sent, 0);
    assert_eq!(snapshot.register_foreign_device.sent, 1);
    assert_eq!(snapshot.foreign_registration.attempts, 1);
    {
        let read = transport.read_bdt(&bbmd);
        tokio::pin!(read);
        let mut packet = [0; 64];
        let (len, source) = tokio::select! {
            result = &mut read => panic!("request finished early: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        assert_eq!(&packet[..len], &[0x81, 2, 0, 4]);
        peer.send_to(&[0x81, 3, 0, 4], source).await.unwrap();
        assert!(read.await.unwrap().is_empty());
    }
    transport.stop().await.unwrap();
}

/// No device could register with a BBMD at the unspecified address, a
/// group address, port 0 or the transport's own address, so `start()`
/// refuses one and binds nothing that stays.
#[tokio::test]
async fn foreign_mode_refuses_a_bbmd_no_device_could_register_with() {
    let own = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let own_port = own.local_addr().unwrap().port();
    drop(own);
    for (bbmd_ip, bbmd_port) in [
        (Ipv4Addr::UNSPECIFIED, 47808),
        (Ipv4Addr::BROADCAST, 47808),
        (Ipv4Addr::new(224, 0, 0, 1), 47808),
        (Ipv4Addr::LOCALHOST, 0),
        (Ipv4Addr::LOCALHOST, own_port),
    ] {
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, own_port, Ipv4Addr::BROADCAST);
        transport.register_as_foreign_device(ForeignDeviceConfig {
            bbmd_ip,
            bbmd_port,
            ttl: 60,
            renewal_interval: None,
        });
        let error = transport.start().await.unwrap_err();
        assert!(matches!(error, Error::Encoding(_)), "{error:?}");
        assert!(error.to_string().contains("BBMD"), "{error}");
        assert!(transport.socket.is_none());
        assert!(!transport.bvlc_client_snapshot().running);
    }
}

#[tokio::test]
async fn foreign_silent_attempt_times_out_then_renews_without_claiming_acceptance() {
    let (peer, mut transport) = peer_and_transport().await;
    let _rx = transport.start().await.unwrap();
    let source = registration(&peer).await;
    // Automatic response waiting is bounded by PERIOD, below the manual 3 s.
    let started = tokio::time::Instant::now();
    assert_eq!(registration(&peer).await, source);
    assert!(started.elapsed() < Duration::from_secs(1));
    let snapshot = transport.bvlc_client_snapshot();
    assert_eq!(snapshot.register_foreign_device.timeouts, 1);
    assert_eq!(snapshot.foreign_registration.accepted, 0);
    assert_eq!(snapshot.foreign_registration.refused, 0);
    assert_eq!(snapshot.foreign_registration.last_result, None);
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    observed(&transport, |s| s.foreign_registration.accepted == 1).await;
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn foreign_busy_worker_retries_after_manual_request_finishes() {
    let (peer, mut transport) = peer_and_transport().await;
    let _rx = transport.start().await.unwrap();
    // The first attempt is admitted at start; answer it.
    let source = registration(&peer).await;
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    observed(&transport, |s| s.foreign_registration.accepted == 1).await;
    let target = encode_bip_mac(
        Ipv4Addr::LOCALHOST.octets(),
        peer.local_addr().unwrap().port(),
    );
    {
        // A manual request holds the one slot across the renewal.
        let manual = transport.read_bdt(&target);
        tokio::pin!(manual);
        let mut packet = [0; 64];
        let (len, source) = tokio::select! {
            result = &mut manual => panic!("manual request finished early: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        assert_eq!(&packet[..len], &[0x81, 2, 0, 4]);
        let busy = tokio::select! {
            result = &mut manual => panic!("manual request finished early: {result:?}"),
            busy = observed(&transport, |s| s.register_foreign_device.busy == 1) => busy,
        };
        assert_eq!(busy.register_foreign_device.sent, 1);
        assert_eq!(busy.foreign_registration.attempts, 1);
        assert!(busy.foreign_registration.next_attempt_in.unwrap() <= PERIOD);
        peer.send_to(&[0x81, 3, 0, 4], source).await.unwrap();
        manual.await.unwrap();
    }
    let source = registration(&peer).await;
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    observed(&transport, |s| s.foreign_registration.accepted == 2).await;
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn foreign_stop_during_pending_request_resets_status_and_drop_aborts_workers() {
    let (peer, mut transport) = peer_and_transport().await;
    let _rx = transport.start().await.unwrap();
    registration(&peer).await;
    let management = transport.client_record.clone();
    transport.stop().await.unwrap();
    assert_eq!(
        management.snapshot().foreign_registration.last_outcome,
        None
    );
    assert_eq!(management.snapshot().register_foreign_device.timeouts, 0);
    let _rx = transport.start().await.unwrap();
    registration(&peer).await;
    let worker = transport.node_task.as_ref().unwrap().abort_handle();
    drop(transport);
    assert_eq!(
        management.snapshot().foreign_registration.next_attempt_in,
        None
    );
    assert_eq!(
        management.snapshot().foreign_registration.last_outcome,
        None
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while !worker.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_tiny_interval_silent_peer_has_bounded_attempts() {
    check_tiny_interval_pacing(false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_tiny_interval_busy_slot_has_bounded_attempts() {
    check_tiny_interval_pacing(true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_tiny_interval_local_errors_have_bounded_attempts() {
    check_tiny_interval_pacing(false, true).await;
}

/// The snapshot once the first registration attempt went out, without
/// letting paused time move.
async fn sent_once(transport: &BipTransport) -> BvlcClientSnapshot {
    for _ in 0..100 {
        let snapshot = transport.bvlc_client_snapshot();
        if snapshot.register_foreign_device.sent == 1 {
            return snapshot;
        }
        tokio::task::yield_now().await;
    }
    panic!("the first registration attempt was not sent");
}

#[tokio::test(start_paused = true)]
async fn foreign_tiny_interval_countdown_tracks_the_effective_schedule() {
    let (_peer, mut transport) = peer_and_transport().await;
    transport.foreign_device.as_mut().unwrap().renewal_interval = Some(Duration::from_nanos(1));
    let _rx = transport.start().await.unwrap();
    let first = sent_once(&transport).await;
    // The first attempt is in flight: the next is one effective interval,
    // 100 ms, after it started.
    assert_eq!(first.register_foreign_device.sent, 1);
    assert_eq!(
        first.foreign_registration.next_attempt_in,
        Some(Duration::from_millis(100))
    );
    tokio::time::advance(Duration::from_millis(99)).await;
    tokio::task::yield_now().await;
    let waiting = transport.bvlc_client_snapshot();
    assert_eq!(waiting.register_foreign_device.sent, 1);
    assert_eq!(
        waiting.foreign_registration.next_attempt_in,
        Some(Duration::from_millis(1))
    );
    // It times out at the interval, and the next one goes at once.
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    let second = transport.bvlc_client_snapshot();
    assert_eq!(second.register_foreign_device.timeouts, 1);
    assert_eq!(second.register_foreign_device.sent, 2);
    assert_eq!(
        second.foreign_registration.next_attempt_in,
        Some(Duration::from_millis(100))
    );
    transport.stop().await.unwrap();
}

async fn check_tiny_interval_pacing(busy: bool, local_error: bool) {
    // The node sends to a sink that never answers, and fails each send for
    // the local-error case.
    let sink = send_sink::SendSink::default();
    sink.fail_sends(local_error || busy);
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let socket = Arc::new(BipSocket::recording(socket, sink.clone()));
    let config = ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: 47808,
        ttl: 2,
        renewal_interval: Some(Duration::from_nanos(1)),
    };
    let interval = config.interval().unwrap();
    let own = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let link = test_support::test_link(
        own,
        own,
        bacnet_bip_core::node::Mode::Foreign(config.engine(interval)),
    );
    let record = Arc::new(client_snapshot::ClientRecord::default());
    let node = Arc::new(node::NodeHandle::new(link, socket, Arc::clone(&record)).unwrap());
    let started = tokio::time::Instant::now();
    let worker = tokio::spawn(Arc::clone(&node).drive());
    // For the busy case, the first attempt fails at once and frees the
    // slot, and a manual request then holds it.
    let held_request = if busy {
        let begun = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let target = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 47809);
                let request = Bvll::ReadBroadcastDistributionTable;
                if let Ok(held) = node.begin(target, request, Duration::from_secs(3)) {
                    return held;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the first attempt frees the slot");
        sink.fail_sends(false);
        Some(begun)
    } else {
        None
    };
    tokio::time::sleep(Duration::from_millis(250)).await;
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    let elapsed = started.elapsed();
    let snapshot = node.snapshot();
    let counters = snapshot.register_foreign_device;
    let attempts = counters.sent + counters.busy + counters.local_errors;
    eprintln!("tiny interval busy={busy} local_error={local_error}: {attempts} attempts in {elapsed:?}; counters={counters:?}");
    assert!(attempts > 0, "worker must make progress");
    let limit = elapsed.as_millis() / 100 + 1;
    assert!(
        u128::from(attempts) <= limit,
        "local pacing allows at most one attempt per 100 ms, with one immediate initial attempt"
    );
    assert_eq!(snapshot.foreign_registration.accepted, 0);
    if busy {
        assert_eq!(counters.sent, 0);
        assert_eq!(counters.local_errors, 1);
        assert!(counters.busy > 0);
    } else if local_error {
        assert_eq!(counters.sent, 0);
        assert_eq!(attempts, counters.local_errors);
    } else {
        assert!(counters.timeouts > 0);
        let sent = sink.take();
        assert_eq!(sent[0].bytes, [0x81, 5, 0, 6, 0, 2]);
    }
    drop(held_request);
}
