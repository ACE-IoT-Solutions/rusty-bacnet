use super::*;

supported_unix_item! {
/// Both control messages a BSD hands an IPv4 datagram fit the buffer, and
/// Linux's packet info for either family, each with a receive timestamp.
#[test]
fn the_control_buffer_holds_every_message_a_datagram_carries() {
    let space = |length: usize| unsafe { libc::CMSG_SPACE(length as _) } as usize;
    // The BSDs' link-layer socket address is 20 to 56 octets long.
    let bsd = space(std::mem::size_of::<libc::in_addr>())
        + space(56)
        + space(std::mem::size_of::<libc::timeval>());
    let linux = space(std::mem::size_of::<libc::in6_pktinfo>())
        + space(12)
        + space(std::mem::size_of::<[libc::timespec; 3]>());
    assert!(bsd.max(linux) <= std::mem::size_of::<ControlBuffer>());
}
}

#[tokio::test]
async fn ipv4_loopback_destination_is_reported() {
    let socket = Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap();
    socket.set_nonblocking(true).unwrap();
    let receiver = DestinationReceiver::configure(&socket, IpVersion::V4).unwrap();
    socket
        .bind(&SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0).into())
        .unwrap();
    let socket = UdpSocket::from_std(socket.into()).unwrap();
    let sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    sender
        .send_to(b"v4", socket.local_addr().unwrap())
        .await
        .unwrap();

    let mut buf = [0u8; 8];
    let received = receiver.recv_from(&socket, &mut buf).await.unwrap();
    assert_eq!(&buf[..received.len], b"v4");
    assert_eq!(received.destination, IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert!(received.arrival_index.is_some_and(|index| index != 0));
}

#[tokio::test]
async fn ipv6_loopback_destination_is_reported() {
    let socket = Socket::new(socket2::Domain::IPV6, socket2::Type::DGRAM, None).unwrap();
    socket.set_only_v6(true).unwrap();
    socket.set_nonblocking(true).unwrap();
    let receiver = DestinationReceiver::configure(&socket, IpVersion::V6).unwrap();
    socket
        .bind(&SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0).into())
        .unwrap();
    let socket = UdpSocket::from_std(socket.into()).unwrap();
    let sender = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    sender
        .send_to(b"v6", socket.local_addr().unwrap())
        .await
        .unwrap();

    let mut buf = [0u8; 8];
    let received = receiver.recv_from(&socket, &mut buf).await.unwrap();
    assert_eq!(&buf[..received.len], b"v6");
    assert_eq!(received.destination, IpAddr::V6(Ipv6Addr::LOCALHOST));
    assert!(received.arrival_index.is_some_and(|index| index != 0));
}

#[tokio::test]
async fn oversized_datagram_is_dropped_without_stopping_receive() {
    let socket = Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap();
    socket.set_nonblocking(true).unwrap();
    let receiver = DestinationReceiver::configure(&socket, IpVersion::V4).unwrap();
    socket
        .bind(&SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0).into())
        .unwrap();
    let socket = UdpSocket::from_std(socket.into()).unwrap();
    let sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let destination = socket.local_addr().unwrap();

    sender.send_to(&[0xAA; 64], destination).await.unwrap();
    let mut buf = [0u8; 8];
    assert_eq!(
        receiver
            .recv_from(&socket, &mut buf)
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );

    sender.send_to(b"ok", destination).await.unwrap();
    let received = receiver.recv_from(&socket, &mut buf).await.unwrap();
    assert_eq!(&buf[..received.len], b"ok");
}

#[test]
fn windows_recoverable_receive_errors_are_droppable_invalid_data() {
    assert_eq!(
        classify_recv_error(10040, &[10040, 10054, 10052]).kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        classify_recv_error(10054, &[10040, 10054, 10052]).kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        classify_recv_error(10052, &[10040, 10054, 10052]).kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        classify_recv_error(12345, &[10040, 10054, 10052]).raw_os_error(),
        Some(12345)
    );
}

/// A realtime stamp counts the fraction in its own parts, and refuses a
/// time before the epoch or a fraction of a whole second or more.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
#[test]
fn a_realtime_stamp_counts_seconds_and_parts() {
    assert_eq!(
        stamps::arrival_stamp(2i64, 5i64, 1_000_000_000),
        Some(2_000_000_005)
    );
    assert_eq!(
        stamps::arrival_stamp(2i64, 999_999i64, 1_000_000),
        Some(2_999_999)
    );
    assert_eq!(stamps::arrival_stamp(-1i64, 0i64, 1_000_000_000), None);
    assert_eq!(stamps::arrival_stamp(1i64, 1_000_000i64, 1_000_000), None);
    assert_eq!(stamps::arrival_stamp(1i64, -1i64, 1_000_000), None);
}
