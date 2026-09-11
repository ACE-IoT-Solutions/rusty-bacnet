use super::*;

#[tokio::test]
async fn received_reject_removes_learned_route() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_learned(3000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
    assert!(table.lookup(3000).is_some());

    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(3);
    payload.put_u8(RejectMessageReason::OTHER.to_raw());
    payload.put_u16(3000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let tbl = table.lock().await;
    assert!(tbl.lookup(3000).is_none());
    assert!(tbl.lookup(1000).is_some());
}

#[tokio::test]
async fn received_reject_does_not_remove_direct_route() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);

    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];
    let mut payload = BytesMut::with_capacity(3);
    payload.put_u8(RejectMessageReason::OTHER.to_raw());
    payload.put_u16(1000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let tbl = table.lock().await;
    assert!(tbl.lookup(1000).is_some());
}

#[tokio::test]
async fn who_is_router_with_specific_network() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_direct(3000, 2);

    let table = Arc::new(Mutex::new(table));

    let (tx, mut rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut req_payload = BytesMut::with_capacity(2);
    req_payload.put_u16(2000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw()),
        payload: req_payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let sent = rx.try_recv().unwrap();
    match sent {
        SendRequest::Broadcast { npdu: data, .. } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.is_network_message);
            assert_eq!(
                decoded.message_type,
                Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
            );
            assert_eq!(decoded.payload.len(), 2);
            let net = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
            assert_eq!(net, 2000);
        }
        _ => panic!("Expected Broadcast response for I-Am-Router"),
    }
}

#[tokio::test]
async fn who_is_router_with_unknown_network_no_response() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);

    let table = Arc::new(Mutex::new(table));

    let (tx, mut rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut req_payload = BytesMut::with_capacity(2);
    req_payload.put_u16(9999);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw()),
        payload: req_payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn initialize_routing_table_ack() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);

    let table = Arc::new(Mutex::new(table));

    let (tx, mut rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE.to_raw()),
        payload: Bytes::new(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let sent = rx.try_recv().unwrap();
    match sent {
        SendRequest::Unicast {
            npdu: data, mac, ..
        } => {
            assert_eq!(mac.as_slice(), &[0x0A]);
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.is_network_message);
            assert_eq!(
                decoded.message_type,
                Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK.to_raw())
            );
            assert_eq!(decoded.payload.len(), 9);
            assert_eq!(decoded.payload[0], 2);
        }
        _ => panic!("Expected Unicast response for Init-Routing-Table"),
    }
}

#[tokio::test]
async fn router_busy_does_not_crash() {
    let table = RouterTable::new();
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(4);
    payload.put_u16(1000);
    payload.put_u16(2000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::ROUTER_BUSY_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;
}

#[tokio::test]
async fn router_available_does_not_crash() {
    let table = RouterTable::new();
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(4);
    payload.put_u16(1000);
    payload.put_u16(2000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;
}

#[tokio::test]
async fn i_could_be_router_stores_potential_route() {
    let table = RouterTable::new();
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(3);
    payload.put_u16(5000);
    payload.put_u8(50);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A, 0x0B], &npdu).await;

    let tbl = table.lock().await;
    let entry = tbl.lookup(5000).unwrap();
    assert!(!entry.directly_connected);
    assert_eq!(entry.port_index, 0);
    assert_eq!(entry.next_hop_mac.as_slice(), &[0x0A, 0x0B]);
}

#[tokio::test]
async fn i_could_be_router_does_not_overwrite_existing_route() {
    let mut table = RouterTable::new();
    table.add_direct(5000, 1);
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(3);
    payload.put_u16(5000);
    payload.put_u8(50);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let tbl = table.lock().await;
    let entry = tbl.lookup(5000).unwrap();
    assert!(entry.directly_connected);
    assert_eq!(entry.port_index, 1);
}

#[tokio::test]
async fn establish_connection_does_not_crash() {
    let table = RouterTable::new();
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(3);
    payload.put_u16(6000);
    payload.put_u8(30);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::ESTABLISH_CONNECTION_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;
}

#[tokio::test]
async fn disconnect_removes_learned_route() {
    let mut table = RouterTable::new();
    table.add_learned(7000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(2);
    payload.put_u16(7000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let tbl = table.lock().await;
    assert!(tbl.lookup(7000).is_none());
}

#[tokio::test]
async fn disconnect_does_not_remove_direct_route() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    let table = Arc::new(Mutex::new(table));

    let (tx, _rx) = mpsc::channel::<SendRequest>(256);
    let send_txs = vec![tx];

    let mut payload = BytesMut::with_capacity(2);
    payload.put_u16(1000);

    let npdu = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    handle_network_message(&table, &send_txs, 0, 1000, &[0x0A], &npdu).await;

    let tbl = table.lock().await;
    assert!(tbl.lookup(1000).is_some());
    assert!(tbl.lookup(1000).unwrap().directly_connected);
}
