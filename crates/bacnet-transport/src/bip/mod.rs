//! BACnet/IP over UDP transport (Annex J).
//!
//! Wraps a tokio UDP socket with BVLL framing. The recv loop decodes
//! incoming BVLL frames and extracts NPDU bytes + source MAC for the
//! network layer. Optionally acts as a BBMD or foreign device.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{debug, warn};

use crate::bbmd::{self, BbmdState, BdtEntry, FdtEntryWire};
pub use crate::bbmd::{FdtCounters, ForeignDevicePolicy};
use crate::bvll::{decode_bip_mac, decode_bvll, encode_bip_mac, encode_bvll, BvllMessage};
use crate::port::{ReceivedNpdu, TransportPort};
use crate::udp_metadata::{DestinationReceiver, IpVersion};
use bacnet_types::enums::{BvlcFunction, BvlcResultCode};
use bacnet_types::error::Error;

mod control;
mod fanout;
mod foreign_device;
mod io;
mod policy_bridge;
mod rate_limit;
pub use control::{BbmdControl, BbmdControlError, BbmdLifecycle, BbmdSnapshot, BbmdSnapshotReader};
pub use fanout::{FanoutCounters, FanoutPolicy};
pub use foreign_device::{
    ForeignDeviceRegistrationHandle, ForeignDeviceRegistrationState,
    ForeignDeviceRegistrationStatus,
};
use io::{handle_bvll_message, original_destination_matches, resolve_local_ip, RecvContext};
pub use policy_bridge::{
    evaluate_narrowing_policy, BvllPolicy, BvllPolicyContext, BvllPolicyVerdict,
    EvaluatedBvllPolicy,
};
pub use rate_limit::ManagementCounters;
use rate_limit::ManagementRateLimiter;

/// Default BACnet/IP port (0xBAC0 = 47808).
pub const DEFAULT_BACNET_PORT: u16 = 0xBAC0;

/// Resolve the OS interface name and index that owns an IPv4 address.
///
/// B/IP sockets bind to `INADDR_ANY` so subnet broadcasts remain visible. A
/// kernel interface binding prevents two shared-port attachments from
/// receiving traffic for one another's interfaces.
#[allow(unsafe_code)]
#[cfg(unix)]
fn resolve_ipv4_interface(addr: Ipv4Addr) -> Option<(Vec<u8>, u32)> {
    use std::ffi::CStr;

    struct IfAddrsGuard(*mut libc::ifaddrs);

    impl Drop for IfAddrsGuard {
        fn drop(&mut self) {
            // SAFETY: getifaddrs allocated this list and this guard owns it.
            unsafe { libc::freeifaddrs(self.0) }
        }
    }

    // SAFETY: pointers are null-checked, sockaddr_in is read only for AF_INET,
    // and interface names are kernel-provided NUL-terminated strings.
    unsafe {
        let mut ifaddrs: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifaddrs) != 0 {
            return None;
        }
        let _guard = IfAddrsGuard(ifaddrs);
        let mut cursor = ifaddrs;
        while !cursor.is_null() {
            let ifa = &*cursor;
            if !ifa.ifa_addr.is_null() && (*ifa.ifa_addr).sa_family as i32 == libc::AF_INET {
                let socket_addr = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                let candidate = Ipv4Addr::from(u32::from_be(socket_addr.sin_addr.s_addr));
                if candidate == addr {
                    let name = CStr::from_ptr(ifa.ifa_name);
                    let index = libc::if_nametoindex(name.as_ptr());
                    if index != 0 {
                        return Some((name.to_bytes().to_vec(), index));
                    }
                }
            }
            cursor = ifa.ifa_next;
        }
        None
    }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "cygwin",
        target_os = "nuttx",
        target_os = "wasi"
    ))
))]
fn ensure_reuse_port_supported(_enabled: bool) -> Result<(), Error> {
    Ok(())
}

#[cfg(not(all(
    unix,
    not(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "cygwin",
        target_os = "nuttx",
        target_os = "wasi"
    ))
)))]
fn ensure_reuse_port_supported(enabled: bool) -> Result<(), Error> {
    if enabled {
        Err(Error::Transport(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "SO_REUSEPORT is unsupported on this platform",
        )))
    } else {
        Ok(())
    }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "cygwin",
        target_os = "nuttx",
        target_os = "wasi"
    ))
))]
fn set_socket_reuse_port(socket: &socket2::Socket, enabled: bool) -> std::io::Result<()> {
    socket.set_reuse_port(enabled)
}

#[cfg(not(all(
    unix,
    not(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "cygwin",
        target_os = "nuttx",
        target_os = "wasi"
    ))
)))]
fn set_socket_reuse_port(_socket: &socket2::Socket, enabled: bool) -> std::io::Result<()> {
    if enabled {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "SO_REUSEPORT is unsupported on this platform",
        ))
    } else {
        Ok(())
    }
}

// Linux permits port sharing when every participant sets SO_REUSEADDR even
// without SO_REUSEPORT, so both options are coupled to the explicit opt-in.
#[cfg(any(target_os = "android", target_os = "linux"))]
fn set_socket_reuse_address(socket: &socket2::Socket, reuse_port: bool) -> std::io::Result<()> {
    socket.set_reuse_address(reuse_port)
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn set_socket_reuse_address(socket: &socket2::Socket, _reuse_port: bool) -> std::io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{
        setsockopt, WSAGetLastError, SOCKET_ERROR, SOL_SOCKET, SO_EXCLUSIVEADDRUSE,
    };

    socket.set_reuse_address(false)?;
    let enabled: i32 = 1;
    // SAFETY: the socket is live and the option value has the expected size
    // for the duration of this synchronous call.
    let result = unsafe {
        setsockopt(
            socket.as_raw_socket() as usize,
            SOL_SOCKET,
            SO_EXCLUSIVEADDRUSE,
            (&enabled as *const i32).cast(),
            std::mem::size_of_val(&enabled) as i32,
        )
    };
    if result == SOCKET_ERROR {
        // SAFETY: setsockopt just failed on this thread.
        Err(std::io::Error::from_raw_os_error(unsafe {
            WSAGetLastError()
        }))
    } else {
        Ok(())
    }
}

// Apple platforms need SO_REUSEADDR for broadcast binds; SO_REUSEPORT gates
// sharing the exact wildcard address and port. Other supported Unix targets
// retain the established broadcast socket behavior.
#[cfg(not(any(target_os = "android", target_os = "linux", windows)))]
fn set_socket_reuse_address(socket: &socket2::Socket, _reuse_port: bool) -> std::io::Result<()> {
    socket.set_reuse_address(true)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BvlcResponseKind {
    WriteBroadcastDistributionTableResult,
    RegisterForeignDeviceResult,
    DeleteForeignDeviceTableEntryResult,
    ReadBroadcastDistributionTableAck,
    ReadForeignDeviceTableAck,
}

impl BvlcResponseKind {
    pub(super) fn accepts(self, msg: &BvllMessage) -> bool {
        if msg.function == BvlcFunction::BVLC_RESULT {
            let Ok(code) = decode_bvlc_result_code(msg) else {
                return true;
            };
            return match self {
                Self::WriteBroadcastDistributionTableResult => matches!(
                    code,
                    BvlcResultCode::SUCCESSFUL_COMPLETION
                        | BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK
                ),
                Self::RegisterForeignDeviceResult => matches!(
                    code,
                    BvlcResultCode::SUCCESSFUL_COMPLETION
                        | BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
                ),
                Self::DeleteForeignDeviceTableEntryResult => matches!(
                    code,
                    BvlcResultCode::SUCCESSFUL_COMPLETION
                        | BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
                ),
                Self::ReadBroadcastDistributionTableAck => {
                    code == BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK
                }
                Self::ReadForeignDeviceTableAck => {
                    code == BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK
                }
            };
        }
        match self {
            Self::WriteBroadcastDistributionTableResult
            | Self::RegisterForeignDeviceResult
            | Self::DeleteForeignDeviceTableEntryResult => false,
            Self::ReadBroadcastDistributionTableAck => {
                msg.function == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK
            }
            Self::ReadForeignDeviceTableAck => {
                msg.function == BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK
            }
        }
    }
}

pub(super) struct PendingBvlcResponse {
    id: u64,
    target: ([u8; 4], u16),
    expected: BvlcResponseKind,
    tx: oneshot::Sender<BvllMessage>,
}

impl PendingBvlcResponse {
    pub(super) fn matches(&self, sender: ([u8; 4], u16), msg: &BvllMessage) -> bool {
        self.target == sender && self.expected.accepts(msg)
    }
}

struct PendingBvlcCleanup {
    pending: Arc<StdMutex<Option<PendingBvlcResponse>>>,
    quarantine: Arc<StdMutex<HashMap<([u8; 4], u16), Instant>>>,
    target: ([u8; 4], u16),
    request_id: u64,
    armed: bool,
}

impl PendingBvlcCleanup {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingBvlcCleanup {
    fn drop(&mut self) {
        let mut slot = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot
            .as_ref()
            .is_some_and(|pending| pending.id == self.request_id)
        {
            *slot = None;
        }
        if self.armed {
            let quiet_until = Instant::now() + BipTransport::BVLC_RESPONSE_TIMEOUT;
            self.quarantine
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entry(self.target)
                .and_modify(|deadline| *deadline = (*deadline).max(quiet_until))
                .or_insert(quiet_until);
        }
    }
}

pub(super) fn expect_bvlc_function(msg: &BvllMessage, expected: BvlcFunction) -> Result<(), Error> {
    if msg.function == expected {
        Ok(())
    } else {
        Err(Error::Encoding(format!(
            "expected BVLC response {expected:?}, got {:?}",
            msg.function
        )))
    }
}

pub(super) fn decode_bvlc_result_code(msg: &BvllMessage) -> Result<BvlcResultCode, Error> {
    expect_bvlc_function(msg, BvlcFunction::BVLC_RESULT)?;
    if msg.payload.len() != std::mem::size_of::<u16>() {
        return Err(Error::Encoding(format!(
            "BVLC-Result payload must be 2 bytes, got {}",
            msg.payload.len()
        )));
    }

    Ok(BvlcResultCode::from_raw(u16::from_be_bytes([
        msg.payload[0],
        msg.payload[1],
    ])))
}

fn bvlc_result_error(msg: &BvllMessage) -> Error {
    match decode_bvlc_result_code(msg) {
        Ok(code) => Error::Bvlc { result_code: code },
        Err(err) => err,
    }
}

fn recoverable_udp_receive_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
    )
}

include!("transport_impl.rs");
include!("port_impl.rs");

#[cfg(test)]
mod acl_tests;
#[cfg(test)]
mod bdt_persistence_tests;
#[cfg(test)]
mod dbtn_tests;
#[cfg(test)]
mod fanout_tests;
#[cfg(test)]
mod fdt_tests;
#[cfg(test)]
mod forwarded_tests;
#[cfg(test)]
mod management_ack_tests;
#[cfg(test)]
mod npdu_addressing_tests;
#[cfg(test)]
mod original_tests;
#[cfg(test)]
mod rate_limit_tests;
#[cfg(test)]
mod response_amplification_tests;
#[cfg(test)]
mod socket_config_tests;
#[cfg(test)]
mod tests;
