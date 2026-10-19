//! Copy local addresses while the OS getifaddrs allocation is alive.

use std::{io, net::Ipv6Addr};

use super::{Candidate, SelectedLink};

#[allow(unsafe_code)] // getifaddrs, the list walk and freeifaddrs
pub(super) fn enumerate() -> io::Result<Vec<Candidate>> {
    struct Addresses(*mut libc::ifaddrs);
    impl Drop for Addresses {
        fn drop(&mut self) {
            // SAFETY: the guard is built only from the list a successful
            // getifaddrs returned, and it alone frees it, exactly once, after
            // the walk's last read.
            unsafe { libc::freeifaddrs(self.0) };
        }
    }
    let mut head = std::ptr::null_mut();
    // SAFETY: `head` is a live, writable pointer slot; getifaddrs sets it to
    // the list it allocates when it returns 0, and the guard frees that list.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let _addresses = Addresses(head);
    let mut candidates = Vec::new();
    let mut current = head;
    while !current.is_null() {
        // SAFETY: `current` is non-null and is the head or an `ifa_next` link
        // of the list getifaddrs returned, which the guard keeps alive.
        let entry = unsafe { &*current };
        if !entry.ifa_addr.is_null() && !entry.ifa_name.is_null() {
            // SAFETY: `ifa_addr` is non-null (checked above) and points to a
            // socket address in the live list, so its family field is readable.
            if unsafe { (*entry.ifa_addr).sa_family as i32 } == libc::AF_INET6 {
                // SAFETY: an AF_INET6 address in the list is a whole
                // sockaddr_in6, suitably aligned by getifaddrs, and its fields
                // are integers. `ifa_name` is non-null (checked above) and a
                // NUL-terminated string in the live list; if_nametoindex only
                // reads it.
                let (address, index) = unsafe {
                    (
                        &*(entry.ifa_addr as *const libc::sockaddr_in6),
                        libc::if_nametoindex(entry.ifa_name),
                    )
                };
                candidates.push(Candidate {
                    link: SelectedLink {
                        address: Ipv6Addr::from(address.sin6_addr.s6_addr),
                        index,
                    },
                    up: entry.ifa_flags & libc::IFF_UP as u32 != 0,
                    multicast: entry.ifa_flags & libc::IFF_MULTICAST as u32 != 0,
                    loopback: entry.ifa_flags & libc::IFF_LOOPBACK as u32 != 0,
                });
            }
        }
        current = entry.ifa_next;
    }
    Ok(candidates)
}
