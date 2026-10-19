//! Receive timestamps (#1546): the socket option that asks for them on each
//! OS, and the control message that carries one.

use super::*;

/// The socket option that asks for receive timestamps, where one is read,
/// and the value it is set to.
#[cfg(any(target_os = "linux", target_os = "android"))]
const ARRIVAL_STAMP_OPTION: Option<(libc::c_int, libc::c_int)> = Some((
    libc::SO_TIMESTAMPING,
    (libc::SOF_TIMESTAMPING_RX_SOFTWARE | libc::SOF_TIMESTAMPING_SOFTWARE) as libc::c_int,
));
#[cfg(target_vendor = "apple")]
const ARRIVAL_STAMP_OPTION: Option<(libc::c_int, libc::c_int)> =
    Some((libc::SO_TIMESTAMP_MONOTONIC, 1));
#[cfg(any(
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
const ARRIVAL_STAMP_OPTION: Option<(libc::c_int, libc::c_int)> = Some((libc::SO_TIMESTAMP, 1));
#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
)))]
const ARRIVAL_STAMP_OPTION: Option<(i32, i32)> = None;

/// Ask the kernel to stamp each datagram `udp_socket` receives with the time
/// it arrived, which [`ReceivedDatagram::arrival_stamp`] then carries: the
/// realtime clock in nanoseconds on Linux (the software receive stamp of
/// `SO_TIMESTAMPING`), the monotonic
/// clock in Mach time units on macOS (`SO_TIMESTAMP_MONOTONIC`; the
/// microsecond `SO_TIMESTAMP` gives two datagrams queued back to back the
/// same stamp), and the realtime clock in microseconds on the BSDs
/// (`SO_TIMESTAMP`). Elsewhere, Windows included, it is `Unsupported`. Only
/// a transport that reads several sockets asks for it, to hand up their
/// datagrams in arrival order (#1546).
///
/// Linux stamps a packet as it arrives only while some socket on the host
/// wants stamps, and turns that on a moment after the first one asks.
/// `SO_TIMESTAMPNS` stamps a packet that arrived before then with the time
/// it is read, which would order it by the read; `SO_TIMESTAMPING` leaves it
/// unstamped, so it goes up in read order without claiming more.
pub(crate) fn request_arrival_stamps(udp_socket: &Socket) -> io::Result<()> {
    match ARRIVAL_STAMP_OPTION {
        #[cfg(any(
            target_os = "linux",
            target_os = "android",
            target_vendor = "apple",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        Some((option, value)) => set_option(udp_socket, libc::SOL_SOCKET, option, value),
        _ => {
            let _ = udp_socket;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "receive timestamps are unsupported on this platform",
            ))
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub(super) unsafe fn unix_arrival_stamp_cmsg(
    header: &libc::cmsghdr,
) -> io::Result<Option<CmsgMetadata>> {
    if header.cmsg_type != libc::SCM_TIMESTAMPING {
        return Ok(None);
    }
    // The software stamp, then two hardware ones; zero is no stamp.
    let [time, ..]: [libc::timespec; 3] =
        unsafe { unix_cmsg_payload(header, "short receive timestamp") }?;
    let stamp = arrival_stamp(time.tv_sec, time.tv_nsec, 1_000_000_000).filter(|&stamp| stamp != 0);
    Ok(stamp.map(CmsgMetadata::ArrivalStamp))
}

/// The control message type of a `SO_TIMESTAMP_MONOTONIC` stamp, a `u64` of
/// Mach absolute time, from the macOS SDK's `sys/socket.h` (libc lacks it).
#[cfg(target_vendor = "apple")]
const SCM_TIMESTAMP_MONOTONIC: libc::c_int = 0x04;

#[cfg(target_vendor = "apple")]
pub(super) unsafe fn unix_arrival_stamp_cmsg(
    header: &libc::cmsghdr,
) -> io::Result<Option<CmsgMetadata>> {
    if header.cmsg_type != SCM_TIMESTAMP_MONOTONIC {
        return Ok(None);
    }
    let ticks: u64 = unsafe { unix_cmsg_payload(header, "short receive timestamp") }?;
    Ok(Some(CmsgMetadata::ArrivalStamp(u128::from(ticks))))
}

#[cfg(any(
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
pub(super) unsafe fn unix_arrival_stamp_cmsg(
    header: &libc::cmsghdr,
) -> io::Result<Option<CmsgMetadata>> {
    if header.cmsg_type != libc::SCM_TIMESTAMP {
        return Ok(None);
    }
    let time: libc::timeval = unsafe { unix_cmsg_payload(header, "short receive timestamp") }?;
    Ok(arrival_stamp(time.tv_sec, time.tv_usec, 1_000_000).map(CmsgMetadata::ArrivalStamp))
}

#[cfg(not(any(
    windows,
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
)))]
supported_unix_item! {
pub(super) unsafe fn unix_arrival_stamp_cmsg(_header: &libc::cmsghdr) -> io::Result<Option<CmsgMetadata>> {
    Ok(None)
}
}

/// A realtime stamp of `seconds` and a `fraction` of `per_second` parts,
/// counted in those parts; `None` before the epoch or with a fraction out of
/// range, which no kernel stamp is.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
pub(super) fn arrival_stamp(
    seconds: impl TryInto<u64>,
    fraction: impl TryInto<u64>,
    per_second: u64,
) -> Option<u128> {
    let (seconds, fraction) = (seconds.try_into().ok()?, fraction.try_into().ok()?);
    (fraction < per_second)
        .then(|| u128::from(seconds) * u128::from(per_second) + u128::from(fraction))
}
