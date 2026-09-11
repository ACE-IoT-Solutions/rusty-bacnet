use super::*;

#[test]
fn parse_address_ipv4() {
    let mac = parse_address("192.168.1.100:47808").unwrap();
    assert_eq!(mac.len(), 6);
    assert_eq!(&mac[..4], &[192, 168, 1, 100]);
    assert_eq!(u16::from_be_bytes([mac[4], mac[5]]), 47808);
}

#[test]
fn parse_address_ipv6() {
    let mac = parse_address("[::1]:47808").unwrap();
    assert_eq!(mac.len(), 18);
    // ::1 → 15 zero bytes + 0x01
    assert_eq!(mac[15], 1);
    assert_eq!(u16::from_be_bytes([mac[16], mac[17]]), 47808);
}

#[test]
fn parse_address_ipv6_full() {
    let mac = parse_address("[fe80::1]:47808").unwrap();
    assert_eq!(mac.len(), 18);
    assert_eq!(mac[0], 0xfe);
    assert_eq!(mac[1], 0x80);
}

#[test]
fn parse_address_hex_mac() {
    let mac = parse_address("01:02:03:04:05:06").unwrap();
    assert_eq!(mac, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn parse_address_mstp_peer_boundaries() {
    for mac in [0, 127, 128, 254] {
        assert_eq!(parse_address(&mac.to_string()).unwrap(), vec![mac as u8]);
        assert_eq!(
            parse_address(&format!("mstp:{mac}")).unwrap(),
            vec![mac as u8]
        );
    }
}

#[test]
fn parse_address_rejects_mstp_broadcast_and_out_of_range_peers() {
    for address in ["255", "mstp:255"] {
        let err = parse_address(address).unwrap_err();
        assert_eq!(
            err.to_string(),
            "ValueError: MS/TP peer address 255 is broadcast, not a unicast peer"
        );
    }
    for address in ["256", "mstp:256", "99999999999999999999"] {
        let err = parse_address(address).unwrap_err();
        assert_eq!(
            err.to_string(),
            "ValueError: MS/TP peer address must be in 0..=254"
        );
    }
}

#[test]
fn parse_address_rejects_malformed_and_negative_mstp_peers_explicitly() {
    for address in ["mstp:", "mstp:not-a-number"] {
        let err = parse_address(address).unwrap_err();
        assert_eq!(
            err.to_string(),
            "ValueError: MS/TP peer address must be a decimal integer in 0..=254"
        );
    }
    for address in ["-1", "mstp:-1"] {
        let err = parse_address(address).unwrap_err();
        assert_eq!(
            err.to_string(),
            "ValueError: MS/TP peer address must be in 0..=254"
        );
    }
}

#[test]
fn parse_address_rejects_garbage() {
    assert!(parse_address("not_an_address").is_err());
}

#[test]
fn parse_address_ipv6_missing_bracket() {
    assert!(parse_address("[::1").is_err());
}

#[test]
fn direct_target_preserves_non_default_bip_port() {
    let target = PyDirectTarget::new("192.0.2.10:47809".to_owned()).unwrap();
    assert_eq!(target.address, "192.0.2.10:47809");
    assert_eq!(target.mac, vec![192, 0, 2, 10, 0xba, 0xc1]);
}

#[test]
fn direct_target_rejects_non_unicast_or_non_bip_addresses() {
    for address in [
        "0.0.0.0:47808",
        "224.0.0.1:47808",
        "255.255.255.255:47808",
        "01:02:03:04:05:06",
        "[::1]:47808",
    ] {
        assert!(
            PyDirectTarget::new(address.to_owned()).is_err(),
            "{address}"
        );
    }
}

#[test]
fn routed_target_preserves_router_dnet_and_multibyte_dadr() {
    let target = PyRoutedTarget::new(
        "192.0.2.1:47810".to_owned(),
        200,
        vec![0xde, 0xad, 0xbe, 0xef],
    )
    .unwrap();
    assert_eq!(target.router, "192.0.2.1:47810");
    assert_eq!(target.router_mac, vec![192, 0, 2, 1, 0xba, 0xc2]);
    assert_eq!(target.network, 200);
    assert_eq!(target.address, vec![0xde, 0xad, 0xbe, 0xef]);
}

#[test]
fn routed_target_validates_dnet_and_dadr_bounds() {
    for network in [0, u16::MAX] {
        assert!(PyRoutedTarget::new("192.0.2.1:47808".to_owned(), network, vec![1],).is_err());
    }
    for address in [Vec::new(), vec![1; 256]] {
        assert!(PyRoutedTarget::new("192.0.2.1:47808".to_owned(), 1, address,).is_err());
    }
}
