//! The Unix receive path on libc, for the targets nix doesn't cover here
//! (#1882): illumos, Solaris and NTO, where nix has no `IPV6_RECVPKTINFO`
//! option, and L4Re and Emscripten. They report IPv6 packet info only, with
//! no IPv4 destination and no receive stamps.

use super::*;

/// Ask for IPv6 packet info; IPv4 destination metadata is unsupported here.
pub(super) fn configure_packet_info(udp_socket: &Socket, version: IpVersion) -> io::Result<()> {
    match version {
        IpVersion::V4 => Err(unsupported_ipv4()),
        IpVersion::V6 => set_option(udp_socket, libc::IPPROTO_IPV6, libc::IPV6_RECVPKTINFO, 1),
    }
}

fn unsupported_ipv4() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "IPv4 destination metadata is unsupported on this platform",
    )
}

/// One datagram from `udp_socket` into `buf`: its length, its sender and
/// what its control messages say about it, through a direct `recvmsg`.
#[allow(unsafe_code)] // recvmsg and its control messages
pub(super) fn recv(
    udp_socket: &UdpSocket,
    buf: &mut [u8],
    version: IpVersion,
) -> io::Result<(usize, SocketAddr, PacketInfo)> {
    use std::mem::{size_of, zeroed};
    use std::os::fd::AsRawFd;

    if matches!(version, IpVersion::V4) {
        return Err(unsupported_ipv4());
    }
    // SAFETY: sockaddr_storage is a plain C struct of integers, so all
    // zeros is a valid value.
    let mut peer_storage: libc::sockaddr_storage = unsafe { zeroed() };
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    let mut control = ControlBuffer::new();
    // SAFETY: msghdr is a plain C struct of integers and raw pointers, so
    // all zeros (null pointers, zero lengths) is a valid value.
    let mut message: libc::msghdr = unsafe { zeroed() };
    message.msg_name = (&mut peer_storage as *mut libc::sockaddr_storage).cast();
    message.msg_namelen = size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.0.as_mut_ptr().cast();
    message.msg_controllen = control.0.len() as _;

    // SAFETY: every pointer in `message` points to live storage of the
    // length given beside it: `peer_storage` (msg_namelen), the iovec over
    // `buf` (iov_len) and `control` (msg_controllen). The kernel writes only
    // within those lengths and keeps no pointer after the synchronous call
    // returns.
    let received = unsafe { libc::recvmsg(udp_socket.as_raw_fd(), &mut message, 0) };
    if received < 0 {
        return Err(io::Error::last_os_error());
    }
    if message.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0 {
        return Err(invalid_metadata("truncated UDP payload or packet metadata"));
    }

    // SAFETY: `peer_storage` is an initialized sockaddr_storage, which is
    // all unix_socket_addr asks.
    let peer = unsafe { unix_socket_addr(&peer_storage) }
        .ok_or_else(|| invalid_metadata("missing or invalid UDP peer address"))?;
    // SAFETY: recvmsg just filled `message`: msg_control still points to
    // the live, 8-aligned `control`, msg_controllen is what the kernel
    // wrote there, and MSG_CTRUNC was rejected above, so every control
    // message lies whole inside that length.
    let info = unsafe { unix_destination(&message) }?;
    Ok((received as usize, peer, info))
}

/// The IPv4 or IPv6 socket address in `storage`, by its family.
///
/// # Safety
///
/// Any initialized `sockaddr_storage` is sound: it is larger than and
/// aligned for both `sockaddr_in` and `sockaddr_in6`, whose fields are all
/// integers, so reading either over it is defined whatever the bytes. The fn
/// is unsafe for the casts alone; callers pass storage the kernel filled.
#[allow(unsafe_code)] // reads sockaddr_storage as sockaddr_in or sockaddr_in6
unsafe fn unix_socket_addr(storage: &libc::sockaddr_storage) -> Option<SocketAddr> {
    match storage.ss_family as libc::c_int {
        libc::AF_INET => {
            // SAFETY: sockaddr_storage is at least as large and as aligned as
            // sockaddr_in, it is initialized, and every sockaddr_in field is
            // an integer, valid for any bytes. The family says it holds one.
            let address = unsafe { &*(storage as *const _ as *const libc::sockaddr_in) };
            Some(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes()),
                u16::from_be(address.sin_port),
            )))
        }
        libc::AF_INET6 => {
            // SAFETY: as above, for sockaddr_in6, which sockaddr_storage is
            // also large and aligned enough to hold.
            let address = unsafe { &*(storage as *const _ as *const libc::sockaddr_in6) };
            Some(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(address.sin6_addr.s6_addr),
                u16::from_be(address.sin6_port),
                address.sin6_flowinfo,
                address.sin6_scope_id,
            )))
        }
        _ => None,
    }
}

/// The IPv6 destination and arrival interface the control messages of
/// `message` carry.
///
/// # Safety
///
/// `message.msg_control` must point to `msg_controllen` initialized bytes,
/// aligned for `cmsghdr`, holding whole control messages: each one's
/// `cmsg_len` must stay inside that length. A `recvmsg` that didn't report
/// `MSG_CTRUNC` leaves the message so; a null or empty control buffer has
/// no messages and is fine too.
#[allow(unsafe_code)] // walks the control messages through raw pointers
unsafe fn unix_destination(message: &libc::msghdr) -> io::Result<PacketInfo> {
    let mut destination = None;
    // SAFETY: CMSG_FIRSTHDR only reads msg_control and msg_controllen and
    // returns null unless a whole cmsghdr fits in the buffer.
    let mut header = unsafe { libc::CMSG_FIRSTHDR(message) };
    while !header.is_null() {
        // SAFETY: a non-null header from CMSG_FIRSTHDR or CMSG_NXTHDR is a
        // whole cmsghdr inside the caller's aligned, initialized buffer.
        let current = unsafe { &*header };
        if current.cmsg_level == libc::IPPROTO_IPV6 && current.cmsg_type == libc::IPV6_PKTINFO {
            // SAFETY: `current` came from CMSG_FIRSTHDR or CMSG_NXTHDR on a
            // message whose control messages lie whole in its buffer (this
            // fn's contract), as in6_pktinfo_payload asks.
            let info = unsafe { in6_pktinfo_payload(current) }?;
            let address = IpAddr::V6(Ipv6Addr::from(info.ipi6_addr.s6_addr));
            if destination.replace((address, info.ipi6_ifindex)).is_some() {
                return Err(invalid_metadata("duplicate UDP destination metadata"));
            }
        }
        // SAFETY: `header` is a control message of `message`, as
        // CMSG_NXTHDR requires; it returns null once the next one would
        // pass the end of the buffer.
        header = unsafe { libc::CMSG_NXTHDR(message, header) };
    }
    let (destination, index) =
        destination.ok_or_else(|| invalid_metadata("missing UDP destination metadata"))?;
    Ok(PacketInfo {
        destination,
        arrival_index: Some(index),
        arrival_stamp: None,
    })
}

/// The `in6_pktinfo` an `IPV6_PKTINFO` control message carries, or
/// `InvalidData` when the message is too short to hold one.
///
/// # Safety
///
/// `header` must be one that `CMSG_FIRSTHDR` or `CMSG_NXTHDR` returned for a
/// message whose control messages lie whole inside its initialized control
/// buffer, so all `cmsg_len` octets from `header` are readable. The kernel
/// leaves a received message so when it doesn't report `MSG_CTRUNC`.
#[allow(unsafe_code)] // reads the payload through CMSG_DATA
unsafe fn in6_pktinfo_payload(header: &libc::cmsghdr) -> io::Result<libc::in6_pktinfo> {
    let size = std::mem::size_of::<libc::in6_pktinfo>();
    // SAFETY: CMSG_LEN is arithmetic on its argument; it touches no memory.
    let required = unsafe { libc::CMSG_LEN(size as _) } as usize;
    #[allow(clippy::unnecessary_cast)] // cmsg_len's type differs by target
    let len = header.cmsg_len as usize;
    if len < required {
        return Err(invalid_metadata("short IPv6 destination metadata"));
    }
    // SAFETY: CMSG_DATA is `header` plus the padded header size. The check
    // above puts CMSG_LEN(size), header, padding and the whole payload,
    // within cmsg_len, which lies inside the readable buffer (the contract),
    // so the read is in bounds. read_unaligned needs no alignment, and
    // in6_pktinfo is plain integers, valid for every bit pattern.
    Ok(unsafe { std::ptr::read_unaligned(libc::CMSG_DATA(header).cast()) })
}
