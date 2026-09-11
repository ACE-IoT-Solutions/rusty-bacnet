#[tokio::test]
async fn foreign_device_broadcast_via_bbmd() {
    // BBMD
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    bbmd_transport.enable_foreign_device_registration(ForeignDevicePolicy::default());
    let mut bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

    // Foreign device
    let mut fd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    fd_transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::from(bbmd_ip),
        bbmd_port,
        ttl: 60,
    });
    let _fd_rx = fd_transport.start().await.unwrap();

    // Give time for registration
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Foreign device sends a broadcast (should use Distribute-Broadcast-To-Network)
    let test_npdu = vec![0x01, 0x00, 0xAA, 0xBB];
    fd_transport.send_broadcast(&test_npdu).await.unwrap();

    // BBMD should receive it (as NPDU via Distribute-Broadcast-To-Network)
    let received = timeout(Duration::from_secs(2), bbmd_rx.recv())
        .await
        .expect("BBMD timed out")
        .expect("BBMD channel closed");

    assert_eq!(received.npdu, test_npdu);

    fd_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn distribute_broadcast_from_unregistered_foreign_device_naks_without_delivery() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let mut bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let response = raw_bvlc_request(
        &bbmd_mac,
        BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
        &[0x01, 0x00, 0xAA, 0xBB],
    )
    .await;
    assert_eq!(
        decode_bvlc_result_code(&response).unwrap(),
        BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK
    );

    assert!(
        timeout(Duration::from_millis(100), bbmd_rx.recv())
            .await
            .is_err(),
        "unregistered DBTN must not deliver an NPDU to the BBMD"
    );

    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn bbmd_management_acl_preserved_after_start() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.enable_bbmd(vec![]);
    transport.set_bbmd_management_acl(vec![[10, 0, 0, 1]]);
    let _rx = transport.start().await.unwrap();

    {
        let state = transport.bbmd_state().unwrap();
        let s = state.lock().await;
        assert!(s.is_management_allowed(&[10, 0, 0, 1]));
        assert!(!s.is_management_allowed(&[10, 0, 0, 2]));
    }

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bvlc_request_rejects_concurrent_calls() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();

    // Manually install a pending sender to simulate an in-flight request
    {
        let (tx, _rx) = oneshot::channel();
        let (ip, port) = decode_bip_mac(transport.local_mac()).unwrap();
        let mut slot = transport.pending_bvlc_response.lock().unwrap();
        *slot = Some(PendingBvlcResponse {
            id: 1,
            target: (ip, port),
            expected: BvlcResponseKind::ReadBroadcastDistributionTableAck,
            tx,
        });
    }

    // A second request should fail immediately
    let fake_target = transport.local_mac().to_vec();
    let result = transport.read_bdt(&fake_target).await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("already in flight"),
        "expected 'already in flight' error, got: {err}"
    );

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn socket_is_broadcast_capable_and_binds_inaddr_any() {
    // Regression for the "user-supplied interface IP" silently rejecting
    // broadcast traffic.  Even when the caller passes a specific interface,
    // the underlying socket must bind 0.0.0.0 so the kernel delivers
    // subnet- and limited-broadcast packets to it.  The interface IP is
    // still used for the announced local MAC.
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();

    let local = transport
        .socket
        .as_ref()
        .expect("socket exists after start")
        .local_addr()
        .expect("local_addr is queryable");

    assert!(
        local.ip().is_unspecified(),
        "BIP socket must bind to 0.0.0.0 for broadcast reception; got {local}"
    );
    assert!(
        socket2::SockRef::from(
            transport
                .socket
                .as_ref()
                .expect("socket exists after start")
                .as_ref()
        )
        .broadcast()
        .expect("SO_BROADCAST is queryable"),
        "BIP socket must enable SO_BROADCAST for Original-Broadcast-NPDU sends"
    );

    // The announced local MAC must still reflect the user-supplied interface,
    // not the bind address.
    let mac = transport.local_mac();
    assert_eq!(
        &mac[..4],
        &Ipv4Addr::LOCALHOST.octets(),
        "announced IP must match interface"
    );

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn start_fails_on_nonlocal_interface() {
    // Now that we bind the real socket to 0.0.0.0, a typo'd interface IP
    // would otherwise succeed at bind and only fail silently later when
    // peers reply to an address we don't own.  start() must instead probe
    // the configured interface and fail fast.  192.0.2.0/24 (RFC 5737
    // TEST-NET-1) is reserved and never assignable to a real NIC, so the
    // probe must reject it.
    let mut transport = BipTransport::new(Ipv4Addr::new(192, 0, 2, 1), 0, Ipv4Addr::BROADCAST);
    let err = transport
        .start()
        .await
        .expect_err("start() must reject a non-local interface IP");
    assert!(
        matches!(err, Error::Transport(_)),
        "expected Error::Transport, got: {err:?}"
    );
}

/// #360: Clause J.1.2's B/IP broadcast address is the configured broadcast
/// IP together with this port's UDP port — a broadcast IP at a different
/// port is a different B/IP network, and the limited broadcast is not this
/// link's spelling unless it is the configured one.
#[test]
fn is_broadcast_mac_requires_configured_ip_and_port() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0xBAC0, Ipv4Addr::new(192, 168, 1, 255));
    assert!(transport.is_broadcast_mac(&[192, 168, 1, 255, 0xBA, 0xC0]));
    assert!(!transport.is_broadcast_mac(&[192, 168, 1, 255, 0xBA, 0xC1]));
    assert!(!transport.is_broadcast_mac(&[255, 255, 255, 255, 0xBA, 0xC0]));
    assert!(!transport.is_broadcast_mac(&[192, 168, 1, 7, 0xBA, 0xC0]));
    assert!(!transport.is_broadcast_mac(&[192, 168, 1, 255]));
}
