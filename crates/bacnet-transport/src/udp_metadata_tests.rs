use super::*;

supported_unix_item! {
/// Both control messages a BSD hands an IPv4 datagram fit the buffer, and
/// Linux's packet info for either family, each with a receive timestamp.
#[test]
#[allow(unsafe_code)] // CMSG_SPACE is an unsafe fn in libc
fn the_control_buffer_holds_every_message_a_datagram_carries() {
    // SAFETY: CMSG_SPACE is arithmetic on its argument; it touches no memory.
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

/// A datagram already queued reads without waiting, though the reactor has
/// not run since it arrived: the read asks the kernel, not tokio's cached
/// readiness, which still says not ready here (#1546).
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn a_queued_datagram_reads_before_the_reactor_sees_it() {
    let socket = Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap();
    socket.set_nonblocking(true).unwrap();
    let receiver = DestinationReceiver::configure(&socket, IpVersion::V4).unwrap();
    socket
        .bind(&SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0).into())
        .unwrap();
    let socket = UdpSocket::from_std(socket.into()).unwrap();
    let sender = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    sender
        .send_to(b"queued", socket.local_addr().unwrap())
        .unwrap();
    // Blocks the only runtime thread, so the reactor can't record readiness.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut buf = [0u8; 8];
    let received = receiver.recv_queued(&socket, &mut buf).unwrap();
    assert_eq!(&buf[..received.len], b"queued");
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

/// A receive stamp too short to read is no stamp: it doesn't fail the read,
/// so the datagram still goes up, in read order (#1546). A whole one reads.
#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
#[test]
#[allow(unsafe_code)] // builds a control message by hand
fn a_short_receive_stamp_is_no_stamp() {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let (kind, whole) = (
        libc::SCM_TIMESTAMPING,
        std::mem::size_of::<[libc::timespec; 3]>(),
    );
    #[cfg(target_vendor = "apple")]
    let (kind, whole) = (stamps::SCM_TIMESTAMP_MONOTONIC, std::mem::size_of::<u64>());
    let stamp = |payload: usize| {
        let mut control = [0u64; 16];
        // SAFETY: `control` is u64-aligned, initialized and 128 octets, so
        // the header and a payload of up to a whole stamp (24 octets at
        // most) lie inside it and cmsg_len stays within it, as
        // unix_arrival_stamp_cmsg asks; the payload bytes stay zero but for
        // one, a valid stamp of any of the plain-integer types read.
        unsafe {
            let header = control.as_mut_ptr().cast::<libc::cmsghdr>();
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = kind;
            (*header).cmsg_len = libc::CMSG_LEN(payload as _) as _;
            *libc::CMSG_DATA(header) = 7;
            stamps::unix_arrival_stamp_cmsg(&*header).map(|m| match m {
                Some(CmsgMetadata::ArrivalStamp(stamp)) => Some(stamp),
                _ => None,
            })
        }
    };
    assert_eq!(stamp(whole - 1).unwrap(), None);
    assert!(stamp(whole).unwrap().is_some());
}
