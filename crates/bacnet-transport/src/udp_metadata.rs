//! Destination-aware UDP receive support for BACnet/IP transports.
//!
//! Most Unix targets receive through nix's safe `recvmsg` and socket options
//! (`udp_metadata_nix.rs`, #1882); the few nix doesn't cover keep the libc
//! code (`udp_metadata_libc.rs`). Windows goes through Winsock here. Each item
//! that still needs unsafe carries its own `#[allow(unsafe_code)]`.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
// Windows, the libc receive path and the tests build these.
#[cfg_attr(not(any(windows, test)), allow(unused_imports))]
use std::net::{SocketAddrV4, SocketAddrV6};

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

/// The Unix receive path on nix (#1882), where nix sets every socket option
/// it needs and decodes or hands over its control messages.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd",
))]
#[path = "udp_metadata_nix.rs"]
mod unix;

/// The Unix receive path on libc, where nix has no IPv6 packet-info socket
/// option.
#[cfg(any(
    target_os = "l4re",
    target_os = "emscripten",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "nto",
))]
#[path = "udp_metadata_libc.rs"]
mod unix;

/// Room for the control messages one IPv4 or IPv6 datagram carries: packet
/// info on Linux and Windows, or a destination address and a whole
/// link-layer address (`IP_RECVIF`) on the BSDs, whose padding can take 84
/// octets on a 32-bit BSD, plus a receive timestamp where one was asked for.
/// Its alignment covers any `cmsghdr`.
///
/// It is also what keeps nix's decoding in bounds. nix reads each message
/// it decodes as a whole struct, without checking `cmsg_len`: `IP_RECVIF`,
/// for one, reads a full `sockaddr_dl` (54 octets on FreeBSD) where the
/// kernel sends about 17. So the buffer must be zeroed for each receive and
/// stay large enough for the messages plus the largest struct nix decodes
/// here, never smaller; such a read then lands on zeros inside it, and a
/// malformed message gives wrong values rather than an out-of-bounds read.
/// `the_control_buffer_holds_every_message_a_datagram_carries` checks the
/// size against whole-struct payloads.
#[cfg(any(unix, windows))]
#[repr(C, align(8))]
struct ControlBuffer([u8; 256]);

#[cfg(any(unix, windows))]
impl ControlBuffer {
    fn new() -> Self {
        Self([0; 256])
    }
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

supported_unix_item! {
/// What a datagram's control messages say about it, taken together.
#[derive(Debug, PartialEq, Eq)]
struct PacketInfo {
    destination: IpAddr,
    arrival_index: Option<u32>,
    arrival_stamp: Option<u128>,
}
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
    /// One direct `recvmsg` on `udp_socket`, never through tokio's readiness.
    fn try_recv_from(
        &self,
        udp_socket: &UdpSocket,
        buf: &mut [u8],
    ) -> io::Result<ReceivedDatagram> {
        let (len, peer, info) = unix::recv(udp_socket, buf, self.version)?;
        Ok(ReceivedDatagram {
            len,
            peer,
            destination: info.destination,
            arrival_index: info.arrival_index,
            os_group_delivery: None,
            arrival_stamp: info.arrival_stamp,
        })
    }
    }

    #[cfg(windows)]
    #[allow(unsafe_code)] // WSARecvMsg
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
        let mut control = ControlBuffer::new();
        let mut message = WSAMSG {
            name: (&mut peer_storage as *mut SOCKADDR_STORAGE).cast(),
            namelen: size_of::<SOCKADDR_STORAGE>() as i32,
            lpBuffers: &mut data,
            dwBufferCount: 1,
            Control: WSABUF {
                len: control.0.len() as u32,
                buf: control.0.as_mut_ptr(),
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

        let control = control
            .0
            .get(..message.Control.len as usize)
            .ok_or_else(|| invalid_metadata("oversized Windows UDP control buffer"))?;
        // SAFETY: `peer_storage` is an initialized SOCKADDR_STORAGE, which is
        // all windows_socket_addr asks.
        let peer = unsafe { windows_socket_addr(&peer_storage) }
            .ok_or_else(|| invalid_metadata("missing or invalid UDP peer address"))?;
        let (destination, arrival_index) = windows_destination(control, self.version)?;
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
fn configure_packet_info(udp_socket: &Socket, version: IpVersion) -> io::Result<()> {
    unix::configure_packet_info(udp_socket, version)
}
}

/// Set the integer socket option `option` at `level` to `value`, for the
/// options nix has no safe setter for: Apple's `SO_TIMESTAMP_MONOTONIC`, and
/// `IPV6_RECVPKTINFO` on the targets that receive through libc.
#[cfg(any(
    target_vendor = "apple",
    target_os = "l4re",
    target_os = "emscripten",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "nto",
))]
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

/// The `N` octets of `bytes` from `offset`, if they are all there.
#[cfg(any(
    windows,
    test,
    target_vendor = "apple",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd",
))]
fn octets_at<const N: usize>(bytes: &[u8], offset: usize) -> Option<[u8; N]> {
    bytes.get(offset..offset.checked_add(N)?)?.try_into().ok()
}

/// The Winsock control-message parser, built on every host for its tests.
#[cfg(any(windows, test))]
#[path = "udp_metadata_winsock.rs"]
mod winsock;
#[cfg(windows)]
use winsock::windows_destination;

#[cfg(test)]
#[path = "udp_metadata_tests.rs"]
mod tests;

#[cfg(all(
    test,
    any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd",
        target_os = "netbsd",
    )
))]
#[path = "udp_metadata_ipv6_tests.rs"]
mod ipv6_tests;
