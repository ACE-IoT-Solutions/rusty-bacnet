//! The crate's operating-system FFI, and the only module in `bacnet-transport`
//! allowed to use `unsafe` (#1923).
//!
//! Everything here is a thin wrapper over one OS call or one read of OS-owned
//! memory: socket options, `recvmsg`/`sendmsg` with hand-built control
//! messages, Winsock's extension functions, the interface lists of
//! `getifaddrs` and `GetAdaptersAddresses`, and the RS-485 ioctl. Each one
//! takes and returns safe types (slices, borrowed sockets and descriptors,
//! socket addresses, plain structs), so the rest of the crate calls them with
//! no unsafe of its own, and each unsafe block keeps its `// SAFETY:`
//! argument beside it.
//!
//! # The rule
//!
//! The workspace denies `unsafe_code`; this module's inner
//! `#![allow(unsafe_code)]` is the crate's only exception. Clippy's
//! `undocumented_unsafe_blocks` and `missing_safety_doc`, and rustc's
//! `unsafe_op_in_unsafe_fn`, still apply inside it. An unsafe fn whose
//! precondition the wrapper can't check stays private to its submodule,
//! behind a safe fn that upholds it. The `unsafe_stays_in_sys` test fails on
//! any `unsafe` token or unsafe-code allow anywhere else in the crate.
//!
//! - `ifaddrs`: the one `getifaddrs` walk (Unix).
//! - `win_adapters`: `GetAdaptersAddresses` (Windows).
//! - `winsock`: Winsock socket options, `WSARecvMsg` and `WSASendMsg`.
//! - `libc_msg`: the libc socket option, `recvmsg` and `sendmsg` paths, for
//!   the Unix targets nix doesn't cover.
//! - `serial`: Linux's `TIOCSRS485` and `TIOCGRS485`.

#![allow(unsafe_code)]

with_getifaddrs! {
    pub(crate) mod ifaddrs;
}

#[cfg(unix)]
pub(crate) mod libc_msg;

#[cfg(all(feature = "serial", any(target_os = "linux", test)))]
pub(crate) mod serial;

#[cfg(windows)]
pub(crate) mod win_adapters;

#[cfg(windows)]
pub(crate) mod winsock;

/// Room for the control messages one IPv4 or IPv6 datagram carries: packet
/// info on Linux and Windows, or a destination address and a whole
/// link-layer address (`IP_RECVIF`) on the BSDs, whose padding can take 84
/// octets on a 32-bit BSD, plus a receive timestamp where one was asked for.
/// Its alignment covers any `cmsghdr`, which the libc control-message walk
/// relies on.
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
pub(crate) struct ControlBuffer(pub(crate) [u8; 256]);

#[cfg(any(unix, windows))]
impl ControlBuffer {
    /// A zeroed buffer, for one receive.
    pub(crate) fn new() -> Self {
        Self([0; 256])
    }
}

#[cfg(test)]
#[path = "fence_tests.rs"]
mod fence_tests;
