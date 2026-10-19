//! Destination-aware UDP receive support for BACnet/IP transports.
//!
//! The receive, socket-option and control-message code here is OS FFI, so
//! each item that needs unsafe carries its own `#[allow(unsafe_code)]`.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

use socket2::Socket;
use tokio::io::Interest;
use tokio::net::UdpSocket;

macro_rules! supported_unix_item {
    ($item:item) => {
        #[cfg(any(
            target_os = "linux",
            target_os = "l4re",
            target_os = "android",
            target_os = "emscripten",
            target_vendor = "apple",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "solaris",
            target_os = "illumos",
            target_os = "nto",
        ))]
        $item
    };
}

macro_rules! unsupported_udp_metadata_item {
    ($item:item) => {
        #[cfg(not(any(
            windows,
            target_os = "linux",
            target_os = "l4re",
            target_os = "android",
            target_os = "emscripten",
            target_vendor = "apple",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "solaris",
            target_os = "illumos",
            target_os = "nto",
        )))]
        $item
    };
}

#[path = "udp_metadata_stamps.rs"]
mod stamps;
pub(crate) use stamps::request_arrival_stamps;
supported_unix_item! {
use stamps::unix_arrival_stamp_cmsg;
}

supported_unix_item! {
/// Room for the control messages one IPv4 or IPv6 datagram carries: packet
/// info on Linux, or a destination address and a whole link-layer address
/// (`IP_RECVIF`) on the BSDs, whose padding can take 84 octets on a 32-bit
/// BSD, plus a receive timestamp where one was asked for. `u64` keeps it
/// aligned for any `cmsghdr`.
type ControlBuffer = [u64; 32];
}

#[derive(Clone, Copy)]
pub(crate) enum IpVersion {
    V4,
    #[cfg_attr(not(feature = "ipv6"), allow(dead_code))]
    V6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReceivedDatagram {
    pub len: usize,
    pub peer: SocketAddr,
    pub destination: IpAddr,
    /// The index of the interface the datagram arrived on, where the OS
    /// reports it: IPv6 packet info everywhere, and for IPv4 the packet info
    /// on Linux and Windows and `IP_RECVIF` on macOS and the BSDs. It is
    /// reported as given: zero names no interface, and consumers treat it so.
    pub arrival_index: Option<u32>,
    pub os_group_delivery: Option<bool>,
    /// When the kernel received the datagram, where
    /// [`request_arrival_stamps`] was set on its socket and the OS reports
    /// it. Its unit depends on the OS, so it only orders datagrams that
    /// sockets of one host received (#1546).
    pub arrival_stamp: Option<u128>,
}

pub(crate) struct DestinationReceiver {
    version: IpVersion,
    #[cfg(windows)]
    recv_msg: windows_sys::Win32::Networking::WinSock::LPFN_WSARECVMSG,
}

impl DestinationReceiver {
    pub fn configure(socket: &Socket, version: IpVersion) -> io::Result<Self> {
        configure_packet_info(socket, version)?;
        Ok(Self {
            version,
            #[cfg(windows)]
            recv_msg: windows_recv_msg(socket)?,
        })
    }

    pub async fn recv_from(
        &self,
        socket: &UdpSocket,
        buf: &mut [u8],
    ) -> io::Result<ReceivedDatagram> {
        socket
            .async_io(Interest::READABLE, || self.try_recv_from(socket, buf))
            .await
    }

    /// The datagram already queued on `socket`, or `WouldBlock`, without
    /// waiting. On Unix this asks the kernel directly rather than tokio's
    /// cached readiness, which can lag behind a datagram the kernel already
    /// holds; `WouldBlock` leaves that readiness as it was, so a later
    /// [`recv_from`](Self::recv_from) still waits correctly. Windows goes
    /// through tokio, which re-arms its poll there on `WouldBlock`.
    pub fn recv_queued(&self, socket: &UdpSocket, buf: &mut [u8]) -> io::Result<ReceivedDatagram> {
        #[cfg(windows)]
        return socket.try_io(Interest::READABLE, || self.try_recv_from(socket, buf));
        #[cfg(not(windows))]
        self.try_recv_from(socket, buf)
    }

    supported_unix_item! {
    #[allow(unsafe_code)] // recvmsg and its control messages
    fn try_recv_from(
        &self,
        udp_socket: &UdpSocket,
        buf: &mut [u8],
    ) -> io::Result<ReceivedDatagram> {
        use std::mem::{size_of, zeroed};
        use std::os::fd::AsRawFd;

        // SAFETY: sockaddr_storage is a plain C struct of integers, so all
        // zeros is a valid value.
        let mut peer_storage: libc::sockaddr_storage = unsafe { zeroed() };
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr().cast(),
            iov_len: buf.len(),
        };
        let mut control: ControlBuffer = [0; 32];
        // SAFETY: msghdr is a plain C struct of integers and raw pointers, so
        // all zeros (null pointers, zero lengths) is a valid value.
        let mut message: libc::msghdr = unsafe { zeroed() };
        message.msg_name = (&mut peer_storage as *mut libc::sockaddr_storage).cast();
        message.msg_namelen = size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = size_of::<ControlBuffer>() as _;

        // SAFETY: every pointer in `message` points to live storage of the
        // length given beside it: `peer_storage` (msg_namelen), the iovec
        // over `buf` (iov_len) and `control` (msg_controllen). The kernel
        // writes only within those lengths and keeps no pointer after the
        // synchronous call returns.
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
        // the live, u64-aligned `control`, msg_controllen is what the kernel
        // wrote there, and MSG_CTRUNC was rejected above, so every control
        // message lies whole inside that length.
        let info = unsafe { unix_destination(&message, self.version) }?;
        Ok(ReceivedDatagram {
            len: received as usize,
            peer,
            destination: info.destination,
            arrival_index: info.arrival_index,
            os_group_delivery: None,
            arrival_stamp: info.arrival_stamp,
        })
    }
    }

    #[cfg(windows)]
    #[allow(unsafe_code)] // WSARecvMsg and its control messages
    fn try_recv_from(
        &self,
        udp_socket: &UdpSocket,
        buf: &mut [u8],
    ) -> io::Result<ReceivedDatagram> {
        use std::mem::{size_of, zeroed};
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock::*;

        let recv_msg = self
            .recv_msg
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "WSARecvMsg unavailable"))?;
        // SAFETY: SOCKADDR_STORAGE is a plain C struct of integers, so all
        // zeros is a valid value.
        let mut peer_storage: SOCKADDR_STORAGE = unsafe { zeroed() };
        let mut data = WSABUF {
            len: u32::try_from(buf.len()).unwrap_or(u32::MAX),
            buf: buf.as_mut_ptr(),
        };
        let mut control = [0usize; 16];
        let mut message = WSAMSG {
            name: (&mut peer_storage as *mut SOCKADDR_STORAGE).cast(),
            namelen: size_of::<SOCKADDR_STORAGE>() as i32,
            lpBuffers: &mut data,
            dwBufferCount: 1,
            Control: WSABUF {
                len: size_of::<[usize; 16]>() as u32,
                buf: control.as_mut_ptr().cast(),
            },
            dwFlags: 0,
        };
        let mut received = 0u32;
        // SAFETY: `recv_msg` is the WSARecvMsg that WSAIoctl returned for
        // this socket. Every pointer in `message` points to live storage of
        // the length given beside it: `peer_storage` (namelen), `data` over
        // `buf` (len, clamped to u32) and `control` (Control.len). Null
        // OVERLAPPED and completion routine make the call synchronous, so no
        // pointer outlives it.
        let result = unsafe {
            recv_msg(
                udp_socket.as_raw_socket() as SOCKET,
                &mut message,
                &mut received,
                std::ptr::null_mut(),
                None,
            )
        };
        if result == SOCKET_ERROR {
            return Err(classify_recv_error(
                // SAFETY: WSAGetLastError takes no arguments and only reads
                // this thread's last Winsock error code.
                unsafe { WSAGetLastError() },
                &[WSAEMSGSIZE, WSAECONNRESET, WSAENETRESET],
            ));
        }
        if message.dwFlags & (MSG_TRUNC | MSG_CTRUNC) != 0 {
            return Err(invalid_metadata("truncated UDP payload or packet metadata"));
        }

        if message.Control.len as usize > std::mem::size_of_val(&control) {
            return Err(invalid_metadata("oversized Windows UDP control buffer"));
        }
        // SAFETY: `peer_storage` is an initialized SOCKADDR_STORAGE, which is
        // all windows_socket_addr asks.
        let peer = unsafe { windows_socket_addr(&peer_storage) }
            .ok_or_else(|| invalid_metadata("missing or invalid UDP peer address"))?;
        // SAFETY: the check above keeps Control.len within `control`, whose
        // bytes are all initialized (it starts zeroed), so the slice is in
        // bounds; u8 has no alignment needs. windows_destination asks for
        // nothing more: it bounds-checks every read against the slice.
        let (destination, arrival_index) = unsafe {
            windows_destination(
                std::slice::from_raw_parts(
                    control.as_ptr().cast::<u8>(),
                    message.Control.len as usize,
                ),
                self.version,
            )
        }?;
        Ok(ReceivedDatagram {
            len: received as usize,
            peer,
            destination,
            arrival_index,
            os_group_delivery: Some(message.dwFlags & (MSG_BCAST | MSG_MCAST) != 0),
            arrival_stamp: None,
        })
    }

    unsupported_udp_metadata_item! {
    fn try_recv_from(
        &self,
        _udp_socket: &UdpSocket,
        _buf: &mut [u8],
    ) -> io::Result<ReceivedDatagram> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "UDP destination metadata is unsupported on this platform",
        ))
    }
    }
}

fn invalid_metadata(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn classify_recv_error(code: i32, droppable_codes: &[i32]) -> io::Error {
    if droppable_codes.contains(&code) {
        invalid_metadata("droppable UDP receive error")
    } else {
        io::Error::from_raw_os_error(code)
    }
}

supported_unix_item! {
/// Set the integer socket option `option` at `level` to `value`.
#[allow(unsafe_code)] // setsockopt
fn set_option(
    udp_socket: &Socket,
    level: libc::c_int,
    option: libc::c_int,
    value: libc::c_int,
) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::fd::AsRawFd;

    // SAFETY: the option value points to `value`, a live c_int, and the
    // length passed is exactly a c_int's; setsockopt only reads it. A wrong
    // level or option for this socket fails with an error, not UB.
    let result = unsafe {
        libc::setsockopt(
            udp_socket.as_raw_fd(),
            level,
            option,
            (&value as *const libc::c_int).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
}

supported_unix_item! {
fn configure_packet_info(udp_socket: &Socket, version: IpVersion) -> io::Result<()> {
    let (level, option) = match version {
        IpVersion::V4 => ipv4_packet_info_option()?,
        IpVersion::V6 => (libc::IPPROTO_IPV6, libc::IPV6_RECVPKTINFO),
    };
    set_option(udp_socket, level, option, 1)?;
    // IP_RECVDSTADDR carries no interface; IP_RECVIF adds it. Only a B/IP
    // transport sharing its port by address reads it, so without it the
    // socket still works.
    #[cfg(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    if matches!(version, IpVersion::V4) {
        if let Err(error) = set_option(udp_socket, libc::IPPROTO_IP, libc::IP_RECVIF, 1) {
            tracing::debug!(%error, "IP_RECVIF unavailable; arrival interfaces unknown");
        }
    }
    Ok(())
}
}

#[cfg(all(unix, any(target_os = "linux", target_os = "android")))]
fn ipv4_packet_info_option() -> io::Result<(libc::c_int, libc::c_int)> {
    Ok((libc::IPPROTO_IP, libc::IP_PKTINFO))
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
supported_unix_item! {
/// IPv4 destination metadata, which this platform doesn't report.
///
/// # Safety
///
/// None here; it shares the signature (and so the contract) of the
/// per-platform parsers below, so callers treat it alike.
#[allow(unsafe_code)] // unsafe fn, to match the other platforms
unsafe fn unix_ipv4_destination_cmsg(_header: &libc::cmsghdr) -> io::Result<Option<CmsgMetadata>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "IPv4 destination metadata is unsupported on this platform",
    ))
}
}

#[cfg(all(
    unix,
    any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
fn ipv4_packet_info_option() -> io::Result<(libc::c_int, libc::c_int)> {
    Ok((libc::IPPROTO_IP, libc::IP_RECVDSTADDR))
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
fn ipv4_packet_info_option() -> io::Result<(libc::c_int, libc::c_int)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "IPv4 destination metadata is unsupported on this platform",
    ))
}

supported_unix_item! {
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
}

supported_unix_item! {
/// What one control message says about a datagram.
enum CmsgMetadata {
    /// Its destination, with the arrival interface when the same message
    /// carries it.
    Destination(IpAddr, Option<u32>),
    /// Only the arrival interface (`IP_RECVIF`).
    #[cfg_attr(not(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    )), allow(dead_code))]
    Interface(u32),
    /// When the kernel received it ([`request_arrival_stamps`]).
    #[cfg_attr(not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    )), allow(dead_code))]
    ArrivalStamp(u128),
}
}

supported_unix_item! {
/// What a datagram's control messages say about it, taken together.
#[derive(Debug, PartialEq, Eq)]
struct PacketInfo {
    destination: IpAddr,
    arrival_index: Option<u32>,
    arrival_stamp: Option<u128>,
}
}

supported_unix_item! {
/// What the control messages of `message` say about its datagram.
///
/// # Safety
///
/// `message.msg_control` must point to `msg_controllen` initialized bytes,
/// aligned for `cmsghdr`, holding whole control messages: each one's
/// `cmsg_len` must stay inside that length. A `recvmsg` that didn't report
/// `MSG_CTRUNC` leaves the message so; a null or empty control buffer has
/// no messages and is fine too.
#[allow(unsafe_code)] // walks the control messages through raw pointers
unsafe fn unix_destination(message: &libc::msghdr, version: IpVersion) -> io::Result<PacketInfo> {
    let mut destination = None;
    let mut interface = None;
    let mut arrival_stamp = None;
    // SAFETY: CMSG_FIRSTHDR only reads msg_control and msg_controllen and
    // returns null unless a whole cmsghdr fits in the buffer.
    let mut header = unsafe { libc::CMSG_FIRSTHDR(message) };
    while !header.is_null() {
        // SAFETY: a non-null header from CMSG_FIRSTHDR or CMSG_NXTHDR is a
        // whole cmsghdr inside the caller's aligned, initialized buffer.
        let current = unsafe { &*header };
        // SAFETY: `current` came from CMSG_FIRSTHDR or CMSG_NXTHDR on a
        // message whose control messages lie whole in its buffer (this fn's
        // contract), which is what unix_destination_cmsg asks.
        match unsafe { unix_destination_cmsg(current, version) }? {
            Some(CmsgMetadata::Destination(address, index)) => {
                if destination.replace(address).is_some() {
                    return Err(invalid_metadata("duplicate UDP destination metadata"));
                }
                interface = index.or(interface);
            }
            Some(CmsgMetadata::Interface(index)) => interface = Some(index),
            Some(CmsgMetadata::ArrivalStamp(time)) => arrival_stamp = Some(time),
            None => {}
        }
        // SAFETY: `header` is a control message of `message`, as
        // CMSG_NXTHDR requires; it returns null once the next one would
        // pass the end of the buffer.
        header = unsafe { libc::CMSG_NXTHDR(message, header) };
    }
    let destination =
        destination.ok_or_else(|| invalid_metadata("missing UDP destination metadata"))?;
    Ok(PacketInfo {
        destination,
        arrival_index: interface,
        arrival_stamp,
    })
}
}

supported_unix_item! {
/// What one control message says about its datagram, if it is one this
/// receive asked for.
///
/// # Safety
///
/// As [`unix_cmsg_payload`]: `header` must be a control message that
/// `CMSG_FIRSTHDR` or `CMSG_NXTHDR` returned, lying whole (all `cmsg_len`
/// octets) inside its message's initialized control buffer.
#[allow(unsafe_code)] // dispatches to the payload readers
unsafe fn unix_destination_cmsg(
    header: &libc::cmsghdr,
    version: IpVersion,
) -> io::Result<Option<CmsgMetadata>> {
    if header.cmsg_level == libc::SOL_SOCKET {
        // SAFETY: this fn's contract on `header` is the stamp parser's.
        return unsafe { unix_arrival_stamp_cmsg(header) };
    }
    match version {
        // SAFETY: this fn's contract on `header` is the IPv4 parser's.
        IpVersion::V4 => unsafe { unix_ipv4_destination_cmsg(header) },
        IpVersion::V6
            if header.cmsg_level == libc::IPPROTO_IPV6
                && header.cmsg_type == libc::IPV6_PKTINFO =>
        {
            // SAFETY: `header` meets unix_cmsg_payload's contract (this fn's),
            // an IPV6_PKTINFO message carries an in6_pktinfo, and that is
            // plain integers, valid for every bit pattern.
            let info: libc::in6_pktinfo =
                unsafe { unix_cmsg_payload(header, "short IPv6 destination metadata") }?;
            let address = IpAddr::V6(Ipv6Addr::from(info.ipi6_addr.s6_addr));
            Ok(Some(CmsgMetadata::Destination(address, Some(info.ipi6_ifindex))))
        }
        _ => Ok(None),
    }
}
}

supported_unix_item! {
/// The `T` a control message carries, or `short` as the error when the
/// message is too short to hold one. It reads unaligned, so `T` can be any
/// plain C struct.
///
/// # Safety
///
/// - `header` must be one that `CMSG_FIRSTHDR` or `CMSG_NXTHDR` returned
///   for a message whose control messages lie whole inside its initialized
///   control buffer, so all `cmsg_len` octets from `header` are readable.
///   The kernel leaves a received message so when it doesn't report
///   `MSG_CTRUNC`.
/// - `T` must be plain old data, valid for every bit pattern: a C struct of
///   integers, or an array of them. `Copy` alone doesn't promise that
///   (`bool`, enums and references are `Copy`), so each caller names a `T`
///   that does.
#[allow(unsafe_code)] // reads the payload through CMSG_DATA
unsafe fn unix_cmsg_payload<T: Copy>(header: &libc::cmsghdr, short: &'static str) -> io::Result<T> {
    // SAFETY: CMSG_LEN is arithmetic on its argument; it touches no memory.
    let required = unsafe { libc::CMSG_LEN(std::mem::size_of::<T>() as _) } as usize;
    #[allow(clippy::unnecessary_cast)] // cmsg_len is usize on Linux but u32 on macOS and the BSDs
    let len = header.cmsg_len as usize;
    if len < required {
        return Err(invalid_metadata(short));
    }
    // SAFETY: CMSG_DATA is `header` plus the padded header size. The check
    // above puts CMSG_LEN(size_of::<T>()) octets, header and padding and all
    // of T, within cmsg_len, which lies inside the readable buffer (the
    // contract), so the read is in bounds. read_unaligned needs no
    // alignment, and the bytes are a valid T because T is plain old data.
    Ok(unsafe { std::ptr::read_unaligned(libc::CMSG_DATA(header).cast::<T>()) })
}
}

/// The destination and arrival interface an `IP_PKTINFO` message carries.
///
/// # Safety
///
/// As [`unix_cmsg_payload`] for `header`.
#[cfg(any(target_os = "linux", target_os = "android"))]
#[allow(unsafe_code)] // reads an in_pktinfo payload
unsafe fn unix_ipv4_destination_cmsg(header: &libc::cmsghdr) -> io::Result<Option<CmsgMetadata>> {
    if header.cmsg_level != libc::IPPROTO_IP || header.cmsg_type != libc::IP_PKTINFO {
        return Ok(None);
    }
    // SAFETY: `header` meets unix_cmsg_payload's contract (this fn's), the
    // level and type checked above mean the payload is an in_pktinfo, and
    // that is plain integers, valid for every bit pattern.
    let info: libc::in_pktinfo =
        unsafe { unix_cmsg_payload(header, "short IPv4 destination metadata") }?;
    let address = IpAddr::V4(Ipv4Addr::from(info.ipi_addr.s_addr.to_ne_bytes()));
    let index = u32::try_from(info.ipi_ifindex).ok();
    Ok(Some(CmsgMetadata::Destination(address, index)))
}

#[cfg(any(
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
/// The destination (`IP_RECVDSTADDR`) or arrival interface (`IP_RECVIF`)
/// one control message carries.
///
/// # Safety
///
/// As [`unix_cmsg_payload`] for `header`.
#[allow(unsafe_code)] // reads in_addr and sockaddr_dl payloads
unsafe fn unix_ipv4_destination_cmsg(header: &libc::cmsghdr) -> io::Result<Option<CmsgMetadata>> {
    if header.cmsg_level != libc::IPPROTO_IP {
        return Ok(None);
    }
    if header.cmsg_type == libc::IP_RECVIF {
        // A link-layer socket address: length and family octets, then the
        // 16-bit interface index, the same on every BSD.
        // SAFETY: `header` meets unix_cmsg_payload's contract (this fn's).
        // An IP_RECVIF payload is a sockaddr_dl, at least 4 octets long when
        // whole; reading its first 4 as [u8; 4], valid for any bits, is
        // sound, and a shorter one fails the length check instead.
        let data: [u8; 4] = unsafe { unix_cmsg_payload(header, "short IPv4 interface metadata") }?;
        let index = u16::from_ne_bytes([data[2], data[3]]);
        return Ok(Some(CmsgMetadata::Interface(u32::from(index))));
    }
    if header.cmsg_type != libc::IP_RECVDSTADDR {
        return Ok(None);
    }
    // SAFETY: `header` meets unix_cmsg_payload's contract (this fn's), the
    // level and type checked above mean the payload is an in_addr, and that
    // is one integer, valid for every bit pattern.
    let address: libc::in_addr =
        unsafe { unix_cmsg_payload(header, "short IPv4 destination metadata") }?;
    let address = IpAddr::V4(Ipv4Addr::from(address.s_addr.to_ne_bytes()));
    Ok(Some(CmsgMetadata::Destination(address, None)))
}

#[cfg(windows)]
#[allow(unsafe_code)] // setsockopt and WSAGetLastError
fn configure_packet_info(udp_socket: &Socket, version: IpVersion) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::*;

    let enabled = 1u32;
    let (level, option) = match version {
        IpVersion::V4 => (IPPROTO_IP, IP_PKTINFO),
        IpVersion::V6 => (IPPROTO_IPV6, IPV6_PKTINFO),
    };
    // SAFETY: the option value points to `enabled`, a live u32, and the
    // length passed is exactly a u32's; setsockopt only reads it.
    let result = unsafe {
        setsockopt(
            udp_socket.as_raw_socket() as SOCKET,
            level,
            option,
            (&enabled as *const u32).cast(),
            size_of::<u32>() as i32,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        // SAFETY: WSAGetLastError takes no arguments and only reads this
        // thread's last Winsock error code.
        Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }))
    }
}

unsupported_udp_metadata_item! {
fn configure_packet_info(_udp_socket: &Socket, _version: IpVersion) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "UDP destination metadata is unsupported on this platform",
    ))
}
}

#[cfg(windows)]
#[allow(unsafe_code)] // WSAIoctl extension-function lookup
fn windows_recv_msg(
    udp_socket: &Socket,
) -> io::Result<windows_sys::Win32::Networking::WinSock::LPFN_WSARECVMSG> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::*;

    let mut function: LPFN_WSARECVMSG = None;
    let mut returned = 0u32;
    // SAFETY: the input points to the WSARecvMsg GUID with its exact size,
    // and the output to `function`, a live LPFN_WSARECVMSG (an
    // Option<fn>, which is pointer-sized and null for None) with its exact
    // size; null OVERLAPPED and callback make the call synchronous.
    let result = unsafe {
        WSAIoctl(
            udp_socket.as_raw_socket() as SOCKET,
            SIO_GET_EXTENSION_FUNCTION_POINTER,
            (&WSAID_WSARECVMSG as *const windows_sys::core::GUID).cast(),
            size_of::<windows_sys::core::GUID>() as u32,
            (&mut function as *mut LPFN_WSARECVMSG).cast(),
            size_of::<LPFN_WSARECVMSG>() as u32,
            &mut returned,
            std::ptr::null_mut(),
            None,
        )
    };
    if result != 0 {
        // SAFETY: WSAGetLastError takes no arguments and only reads this
        // thread's last Winsock error code.
        return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
    }
    function
        .map(Some)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "WSARecvMsg unavailable"))
}

/// The IPv4 or IPv6 socket address in `storage`, by its family.
///
/// # Safety
///
/// Any initialized `SOCKADDR_STORAGE` is sound: it is larger than and
/// aligned for both `SOCKADDR_IN` and `SOCKADDR_IN6`, whose fields and
/// unions are all integers, so reading either over it is defined whatever
/// the bytes. The fn is unsafe for the casts alone.
#[cfg(windows)]
#[allow(unsafe_code)] // reads SOCKADDR_STORAGE as SOCKADDR_IN or SOCKADDR_IN6
unsafe fn windows_socket_addr(
    storage: &windows_sys::Win32::Networking::WinSock::SOCKADDR_STORAGE,
) -> Option<SocketAddr> {
    use windows_sys::Win32::Networking::WinSock::*;

    match storage.ss_family {
        AF_INET => {
            // SAFETY: SOCKADDR_STORAGE is at least as large and as aligned as
            // SOCKADDR_IN, it is initialized, and SOCKADDR_IN is integers,
            // valid for any bytes. The family says it holds one.
            let address = unsafe { &*(storage as *const _ as *const SOCKADDR_IN) };
            // SAFETY: every variant of the IN_ADDR union is integers over the
            // same four initialized octets, so reading S_addr is defined.
            let bytes = unsafe { address.sin_addr.S_un.S_addr.to_ne_bytes() };
            Some(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::from(bytes),
                u16::from_be(address.sin_port),
            )))
        }
        AF_INET6 => {
            // SAFETY: as above, for SOCKADDR_IN6, which SOCKADDR_STORAGE is
            // also large and aligned enough to hold.
            let address = unsafe { &*(storage as *const _ as *const SOCKADDR_IN6) };
            // SAFETY: both IN6_ADDR union variants are integer arrays over the
            // same sixteen initialized octets.
            let bytes = unsafe { address.sin6_addr.u.Byte };
            Some(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(bytes),
                u16::from_be(address.sin6_port),
                address.sin6_flowinfo,
                // SAFETY: both variants of this union (a u32 and the
                // SCOPE_ID struct over it) are integers of the same size.
                unsafe { address.Anonymous.sin6_scope_id },
            )))
        }
        _ => None,
    }
}

/// The destination and arrival interface in a Winsock control buffer.
///
/// # Safety
///
/// None beyond a valid slice: every read is bounds-checked against
/// `control`, whatever its bytes, and none of the checks can overflow. It
/// stays `unsafe` for its unaligned reads alone.
#[cfg(windows)]
#[allow(unsafe_code)] // reads CMSGHDR and packet-info payloads from bytes
unsafe fn windows_destination(
    control: &[u8],
    version: IpVersion,
) -> io::Result<(IpAddr, Option<u32>)> {
    use std::mem::size_of;
    use windows_sys::Win32::Networking::WinSock::*;

    let align = |length: usize| (length + size_of::<usize>() - 1) & !(size_of::<usize>() - 1);
    let data_offset = align(size_of::<CMSGHDR>());
    let mut offset = 0usize;
    let mut destination = None;
    while offset + size_of::<CMSGHDR>() <= control.len() {
        // SAFETY: the loop condition keeps a whole CMSGHDR inside `control`
        // from `offset`; read_unaligned needs no alignment, and CMSGHDR is
        // integers, valid for any bytes.
        let header: CMSGHDR =
            unsafe { std::ptr::read_unaligned(control.as_ptr().add(offset).cast()) };
        // The loop condition keeps `offset` within the buffer, so the
        // subtraction can't underflow, and comparing `cmsg_len` with the
        // octets left can't overflow, whatever `cmsg_len` holds.
        if header.cmsg_len < data_offset || header.cmsg_len > control.len() - offset {
            return Err(invalid_metadata("invalid Windows UDP control message"));
        }
        // SAFETY: data_offset <= cmsg_len <= len - offset (checked above,
        // with no arithmetic that can wrap), so offset + data_offset is at
        // most len: the pointer is within `control` or one past its end.
        let data = unsafe { control.as_ptr().add(offset + data_offset) };
        let candidate = match version {
            IpVersion::V4
                if header.cmsg_level == IPPROTO_IP
                    && header.cmsg_type == IP_PKTINFO
                    && header.cmsg_len >= data_offset + size_of::<IN_PKTINFO>() =>
            {
                // SAFETY: the guard puts data_offset plus an IN_PKTINFO within
                // cmsg_len, inside `control`; read_unaligned needs no
                // alignment, and IN_PKTINFO is integers, valid for any bytes.
                let info: IN_PKTINFO = unsafe { std::ptr::read_unaligned(data.cast()) };
                // SAFETY: every IN_ADDR union variant is integers over the
                // same four initialized octets.
                let bytes = unsafe { info.ipi_addr.S_un.S_addr.to_ne_bytes() };
                Some((IpAddr::V4(Ipv4Addr::from(bytes)), Some(info.ipi_ifindex)))
            }
            IpVersion::V6
                if header.cmsg_level == IPPROTO_IPV6
                    && header.cmsg_type == IPV6_PKTINFO
                    && header.cmsg_len >= data_offset + size_of::<IN6_PKTINFO>() =>
            {
                // SAFETY: the guard puts data_offset plus an IN6_PKTINFO within
                // cmsg_len, inside `control`; read_unaligned needs no
                // alignment, and IN6_PKTINFO is integers, valid for any bytes.
                let info: IN6_PKTINFO = unsafe { std::ptr::read_unaligned(data.cast()) };
                Some((
                    // SAFETY: both IN6_ADDR union variants are integer arrays
                    // over the same sixteen initialized octets.
                    IpAddr::V6(Ipv6Addr::from(unsafe { info.ipi6_addr.u.Byte })),
                    Some(info.ipi6_ifindex),
                ))
            }
            _ => None,
        };
        if let Some(candidate) = candidate {
            if destination.replace(candidate).is_some() {
                return Err(invalid_metadata("duplicate UDP destination metadata"));
            }
        }
        offset = offset.saturating_add(align(header.cmsg_len));
    }
    destination.ok_or_else(|| invalid_metadata("missing UDP destination metadata"))
}

#[cfg(test)]
#[path = "udp_metadata_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "udp_metadata_ipv6_tests.rs"]
mod ipv6_tests;
