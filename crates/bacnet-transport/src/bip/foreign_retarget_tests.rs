//! A running foreign device moved to another BBMD and time-to-live through
//! the registration it lends the Network Port object (#1825).

use super::*;
use bacnet_types::bip_port::ForeignDeviceChanges;
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};

/// Real loopback: generous, since nothing in these tests waits on a timer
/// (the 60 s time-to-live renews at 30 s).
const WAIT: Duration = Duration::from_secs(10);

async fn bbmd() -> UdpSocket {
    UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap()
}

fn foreign_to(peer: &UdpSocket, ttl: u16) -> ForeignDeviceConfig {
    ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: peer.local_addr().unwrap().port(),
        ttl,
        renewal_interval: None,
    }
}

fn address_of(peer: &UdpSocket) -> BACnetHostNPort {
    BACnetHostNPort::from_socket_addr(peer.local_addr().unwrap())
}

/// The Register-Foreign-Device `peer` receives next: its time-to-live and
/// where it came from.
async fn registration(peer: &UdpSocket) -> (u16, std::net::SocketAddr) {
    let mut packet = [0; 64];
    let (len, source) = tokio::time::timeout(WAIT, peer.recv_from(&mut packet))
        .await
        .expect("a registration")
        .unwrap();
    assert_eq!(&packet[..4], &[0x81, 5, 0, 6], "{:?}", &packet[..len]);
    (u16::from_be_bytes([packet[4], packet[5]]), source)
}

async fn observed(
    transport: &BipTransport,
    predicate: impl Fn(&ForeignRegistrationSnapshot) -> bool,
) -> ForeignRegistrationSnapshot {
    tokio::time::timeout(WAIT, async {
        loop {
            let snapshot = transport.bvlc_client_snapshot().foreign_registration;
            if predicate(&snapshot) {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("registration state did not arrive")
}

fn lent(transport: &BipTransport) -> Arc<dyn ForeignRegistration> {
    match transport.bip_port().unwrap().mode {
        BipPortMode::Foreign { registration } => registration,
        other => panic!("{other:?}"),
    }
}

fn changes(bbmd: Option<BACnetHostNPort>, ttl: Option<u16>) -> ForeignDeviceChanges {
    let mut changes = ForeignDeviceChanges::default();
    changes.bbmd = bbmd;
    changes.subscription_lifetime = ttl;
    changes
}

fn is_out_of_range(error: &Error) -> bool {
    matches!(error, Error::Protocol { class, code }
        if *class == ErrorClass::PROPERTY && *code == ErrorCode::VALUE_OUT_OF_RANGE)
}

/// Registered with the first BBMD, the device moves: its next registration
/// goes to the second with the new time-to-live, and the status the TUI and
/// CLI read follows it.
#[tokio::test]
async fn activation_sends_the_next_registration_to_the_new_bbmd_with_the_new_ttl() {
    let (first, second) = (bbmd().await, bbmd().await);
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(foreign_to(&first, 60));
    let _rx = transport.start().await.unwrap();
    let (ttl, source) = registration(&first).await;
    assert_eq!(ttl, 60);
    first.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    observed(&transport, |s| s.registered).await;

    let registration_view = lent(&transport);
    let moved = changes(Some(address_of(&second)), Some(120));
    registration_view.check(&moved).unwrap();
    registration_view.activate(&moved).unwrap();
    assert_eq!(registration_view.bbmd(), address_of(&second));
    assert_eq!(registration_view.subscription_lifetime(), 120);
    assert_eq!(transport.foreign_device_config().unwrap().ttl, 120);

    let (ttl, source) = registration(&second).await;
    assert_eq!(ttl, 120);
    let second_addr = match second.local_addr().unwrap() {
        std::net::SocketAddr::V4(address) => address,
        other => panic!("{other}"),
    };
    // The status moved with it: the newest attempt is to the new BBMD, and
    // the old BBMD's lease no longer counts.
    let moving = observed(&transport, |s| s.last_bbmd == Some(second_addr)).await;
    assert_eq!(moving.last_ttl, Some(120));
    assert_eq!(
        moving.last_outcome,
        Some(ForeignRegistrationOutcome::Pending)
    );
    assert!(!moving.registered);
    assert_eq!(moving.lease_remaining, None);
    second
        .send_to(&[0x81, 0, 0, 6, 0, 0], source)
        .await
        .unwrap();
    let registered = observed(&transport, |s| s.registered).await;
    assert!(registered.lease_remaining.unwrap() > Duration::from_secs(60));
    assert_eq!(registered.accepted, 2);
    // Broadcasts follow: the link the send path reads moved too.
    transport.send_broadcast(&[0x01, 0x00]).await.unwrap();
    let mut packet = [0; 64];
    let (len, _) = tokio::time::timeout(WAIT, second.recv_from(&mut packet))
        .await
        .expect("a distributed broadcast")
        .unwrap();
    assert_eq!(&packet[..len], &[0x81, 0x09, 0, 6, 0x01, 0x00]);

    // A restart keeps the values it moved to.
    transport.stop().await.unwrap();
    let _rx = transport.start().await.unwrap();
    assert_eq!(registration(&second).await.0, 120);
    transport.stop().await.unwrap();
}

/// A value the device couldn't register with is refused by the check and
/// by activation, and changes neither what it reports nor where it
/// registers.
#[tokio::test]
async fn a_refused_value_leaves_the_registration_untouched() {
    let first = bbmd().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(foreign_to(&first, 60));
    let _rx = transport.start().await.unwrap();
    registration(&first).await;
    let view = lent(&transport);
    let before = transport.bvlc_client_snapshot().foreign_registration;
    let own = BACnetHostNPort::from_socket_addr(
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, transport.port).into(),
    );
    let host = |host, port| BACnetHostNPort { host, port };
    let refused = [
        changes(Some(own), None),
        changes(
            Some(host(BACnetHostAddress::Name("bbmd.example".into()), 47808)),
            None,
        ),
        changes(Some(host(BACnetHostAddress::None, 47808)), None),
        changes(
            Some(host(BACnetHostAddress::Ip("::1".parse().unwrap()), 47808)),
            None,
        ),
        changes(
            Some(host(
                BACnetHostAddress::Ip(Ipv4Addr::UNSPECIFIED.into()),
                47808,
            )),
            None,
        ),
        changes(
            Some(host(
                BACnetHostAddress::Ip(Ipv4Addr::BROADCAST.into()),
                47808,
            )),
            None,
        ),
        changes(
            Some(host(BACnetHostAddress::Ip(Ipv4Addr::LOCALHOST.into()), 0)),
            None,
        ),
        changes(None, Some(0)),
    ];
    for refused in refused {
        let error = view.check(&refused).unwrap_err();
        assert!(is_out_of_range(&error), "{refused:?}: {error:?}");
        let error = view.activate(&refused).unwrap_err();
        assert!(is_out_of_range(&error), "{refused:?}: {error:?}");
    }
    assert_eq!(view.bbmd(), address_of(&first));
    assert_eq!(view.subscription_lifetime(), 60);
    assert_eq!(
        transport.foreign_device_config(),
        Some(foreign_to(&first, 60))
    );
    let after = transport.bvlc_client_snapshot().foreign_registration;
    assert_eq!(
        (after.attempts, after.last_bbmd, after.last_ttl),
        (before.attempts, before.last_bbmd, before.last_ttl)
    );
    transport.stop().await.unwrap();
}

/// A renewal interval set for the old time-to-live stays, and a lifetime it
/// doesn't fit under is refused rather than quietly changing it.
#[tokio::test]
async fn a_lifetime_shorter_than_the_configured_renewal_is_refused() {
    let first = bbmd().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut config = foreign_to(&first, 60);
    config.renewal_interval = Some(Duration::from_secs(20));
    transport.register_as_foreign_device(config);
    let view = lent(&transport);
    assert!(is_out_of_range(
        &view.check(&changes(None, Some(20))).unwrap_err()
    ));
    view.check(&changes(None, Some(21))).unwrap();
}

/// Moved before it starts, the device registers at the new BBMD from its
/// first attempt.
#[tokio::test]
async fn a_move_before_start_is_the_first_registration() {
    let (first, second) = (bbmd().await, bbmd().await);
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(foreign_to(&first, 60));
    lent(&transport)
        .activate(&changes(Some(address_of(&second)), Some(90)))
        .unwrap();
    let _rx = transport.start().await.unwrap();
    assert_eq!(registration(&second).await.0, 90);
    // That was the only attempt: none went to the old BBMD.
    let status = transport.bvlc_client_snapshot().foreign_registration;
    assert_eq!(status.attempts, 1);
    assert_eq!(
        status.last_bbmd.map(std::net::SocketAddr::V4),
        Some(second.local_addr().unwrap())
    );
    transport.stop().await.unwrap();
}
