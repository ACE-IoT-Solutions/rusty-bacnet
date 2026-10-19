//! Destination-aware UDP receive support for BACnet/IP transports.
#![allow(unsafe_code)]

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
    fn try_recv_from(
        &self,
        udp_socket: &UdpSocket,
        buf: &mut [u8],
    ) -> io::Result<ReceivedDatagram> {
        use std::mem::{size_of, zeroed};
        use std::os::fd::AsRawFd;

        let mut peer_storage: libc::sockaddr_storage = unsafe { zeroed() };
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr().cast(),
            iov_len: buf.len(),
        };
        let mut control: ControlBuffer = [0; 32];
        let mut message: libc::msghdr = unsafe { zeroed() };
        message.msg_name = (&mut peer_storage as *mut libc::sockaddr_storage).cast();
        message.msg_namelen = size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = size_of::<ControlBuffer>() as _;

        let received = unsafe { libc::recvmsg(udp_socket.as_raw_fd(), &mut message, 0) };
        if received < 0 {
            return Err(io::Error::last_os_error());
        }
        if message.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0 {
            return Err(invalid_metadata("truncated UDP payload or packet metadata"));
        }

        let peer = unsafe { unix_socket_addr(&peer_storage) }
            .ok_or_else(|| invalid_metadata("missing or invalid UDP peer address"))?;
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
        let peer = unsafe { windows_socket_addr(&peer_storage) }
            .ok_or_else(|| invalid_metadata("missing or invalid UDP peer address"))?;
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
fn set_option(
    udp_socket: &Socket,
    level: libc::c_int,
    option: libc::c_int,
    value: libc::c_int,
) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::fd::AsRawFd;

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
unsafe fn unix_socket_addr(storage: &libc::sockaddr_storage) -> Option<SocketAddr> {
    match storage.ss_family as libc::c_int {
        libc::AF_INET => {
            let address = unsafe { &*(storage as *const _ as *const libc::sockaddr_in) };
            Some(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes()),
                u16::from_be(address.sin_port),
            )))
        }
        libc::AF_INET6 => {
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
unsafe fn unix_destination(message: &libc::msghdr, version: IpVersion) -> io::Result<PacketInfo> {
    let mut destination = None;
    let mut interface = None;
    let mut arrival_stamp = None;
    let mut header = unsafe { libc::CMSG_FIRSTHDR(message) };
    while !header.is_null() {
        let current = unsafe { &*header };
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
unsafe fn unix_destination_cmsg(
    header: &libc::cmsghdr,
    version: IpVersion,
) -> io::Result<Option<CmsgMetadata>> {
    if header.cmsg_level == libc::SOL_SOCKET {
        return unsafe { unix_arrival_stamp_cmsg(header) };
    }
    match version {
        IpVersion::V4 => unsafe { unix_ipv4_destination_cmsg(header) },
        IpVersion::V6
            if header.cmsg_level == libc::IPPROTO_IPV6
                && header.cmsg_type == libc::IPV6_PKTINFO =>
        {
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
/// `header` must be one that `CMSG_FIRSTHDR` or `CMSG_NXTHDR` returned for a
/// received message, whose `cmsg_len` the kernel bounded by the control
/// buffer.
unsafe fn unix_cmsg_payload<T: Copy>(header: &libc::cmsghdr, short: &'static str) -> io::Result<T> {
    let required = unsafe { libc::CMSG_LEN(std::mem::size_of::<T>() as _) } as usize;
    #[allow(clippy::unnecessary_cast)] // cmsg_len is usize on Linux but u32 on macOS and the BSDs
    let len = header.cmsg_len as usize;
    if len < required {
        return Err(invalid_metadata(short));
    }
    Ok(unsafe { std::ptr::read_unaligned(libc::CMSG_DATA(header).cast::<T>()) })
}
}

#[cfg(any(target_os = "linux", target_os = "android"))]
unsafe fn unix_ipv4_destination_cmsg(header: &libc::cmsghdr) -> io::Result<Option<CmsgMetadata>> {
    if header.cmsg_level != libc::IPPROTO_IP || header.cmsg_type != libc::IP_PKTINFO {
        return Ok(None);
    }
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
unsafe fn unix_ipv4_destination_cmsg(header: &libc::cmsghdr) -> io::Result<Option<CmsgMetadata>> {
    if header.cmsg_level != libc::IPPROTO_IP {
        return Ok(None);
    }
    if header.cmsg_type == libc::IP_RECVIF {
        // A link-layer socket address: length and family octets, then the
        // 16-bit interface index, the same on every BSD.
        let data: [u8; 4] = unsafe { unix_cmsg_payload(header, "short IPv4 interface metadata") }?;
        let index = u16::from_ne_bytes([data[2], data[3]]);
        return Ok(Some(CmsgMetadata::Interface(u32::from(index))));
    }
    if header.cmsg_type != libc::IP_RECVDSTADDR {
        return Ok(None);
    }
    let address: libc::in_addr =
        unsafe { unix_cmsg_payload(header, "short IPv4 destination metadata") }?;
    let address = IpAddr::V4(Ipv4Addr::from(address.s_addr.to_ne_bytes()));
    Ok(Some(CmsgMetadata::Destination(address, None)))
}

#[cfg(windows)]
fn configure_packet_info(udp_socket: &Socket, version: IpVersion) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::*;

    let enabled = 1u32;
    let (level, option) = match version {
        IpVersion::V4 => (IPPROTO_IP, IP_PKTINFO),
        IpVersion::V6 => (IPPROTO_IPV6, IPV6_PKTINFO),
    };
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
fn windows_recv_msg(
    udp_socket: &Socket,
) -> io::Result<windows_sys::Win32::Networking::WinSock::LPFN_WSARECVMSG> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::*;

    let mut function: LPFN_WSARECVMSG = None;
    let mut returned = 0u32;
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
        return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
    }
    function
        .map(Some)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "WSARecvMsg unavailable"))
}

#[cfg(windows)]
unsafe fn windows_socket_addr(
    storage: &windows_sys::Win32::Networking::WinSock::SOCKADDR_STORAGE,
) -> Option<SocketAddr> {
    use windows_sys::Win32::Networking::WinSock::*;

    match storage.ss_family {
        AF_INET => {
            let address = unsafe { &*(storage as *const _ as *const SOCKADDR_IN) };
            let bytes = unsafe { address.sin_addr.S_un.S_addr.to_ne_bytes() };
            Some(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::from(bytes),
                u16::from_be(address.sin_port),
            )))
        }
        AF_INET6 => {
            let address = unsafe { &*(storage as *const _ as *const SOCKADDR_IN6) };
            let bytes = unsafe { address.sin6_addr.u.Byte };
            Some(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(bytes),
                u16::from_be(address.sin6_port),
                address.sin6_flowinfo,
                unsafe { address.Anonymous.sin6_scope_id },
            )))
        }
        _ => None,
    }
}

#[cfg(windows)]
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
        let header: CMSGHDR =
            unsafe { std::ptr::read_unaligned(control.as_ptr().add(offset).cast()) };
        if header.cmsg_len < data_offset || offset + header.cmsg_len > control.len() {
            return Err(invalid_metadata("invalid Windows UDP control message"));
        }
        let data = unsafe { control.as_ptr().add(offset + data_offset) };
        let candidate = match version {
            IpVersion::V4
                if header.cmsg_level == IPPROTO_IP
                    && header.cmsg_type == IP_PKTINFO
                    && header.cmsg_len >= data_offset + size_of::<IN_PKTINFO>() =>
            {
                let info: IN_PKTINFO = unsafe { std::ptr::read_unaligned(data.cast()) };
                let bytes = unsafe { info.ipi_addr.S_un.S_addr.to_ne_bytes() };
                Some((IpAddr::V4(Ipv4Addr::from(bytes)), Some(info.ipi_ifindex)))
            }
            IpVersion::V6
                if header.cmsg_level == IPPROTO_IPV6
                    && header.cmsg_type == IPV6_PKTINFO
                    && header.cmsg_len >= data_offset + size_of::<IN6_PKTINFO>() =>
            {
                let info: IN6_PKTINFO = unsafe { std::ptr::read_unaligned(data.cast()) };
                Some((
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
