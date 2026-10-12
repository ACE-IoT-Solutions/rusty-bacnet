use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use bacnet_bip_core::node::{ConfigError, Foreign, ForwardedTrust, Groups, Retry};
use bacnet_types::error::Error;

use super::node::core_duration;
use super::BipTransport;

/// Configuration for automatic foreign-device registration. Its positive TTL
/// is sent unchanged; the independent renewal interval controls local attempts.
#[derive(Debug, Clone)]
pub struct ForeignDeviceConfig {
    /// BBMD IP address to register with.
    pub bbmd_ip: Ipv4Addr,
    /// BBMD UDP port.
    pub bbmd_port: u16,
    /// Advertised time-to-live in seconds; must be positive for automatic mode.
    /// The one-shot registration helper separately permits zero-TTL requests.
    pub ttl: u16,
    /// Requested local attempt interval, positive and shorter than `ttl`.
    /// `None` uses half the TTL, including 500 ms for a one-second TTL.
    /// Positive values below 100 ms use 100 ms to bound local retry frequency.
    /// This pacing floor is local policy and does not change the wire TTL.
    pub renewal_interval: Option<Duration>,
}

impl ForeignDeviceConfig {
    pub(super) fn interval(&self) -> Result<Duration, Error> {
        let ttl = Duration::from_secs(u64::from(self.ttl));
        let interval = self.renewal_interval.unwrap_or(ttl / 2);
        if self.ttl == 0 || interval.is_zero() || interval >= ttl {
            return Err(Error::Encoding(
                "foreign-device mode needs TTL > 0 and a renewal interval > 0 and < TTL".into(),
            ));
        }
        // Limit fast replies, busy slots and local failures to ten attempts per
        // second. This remains below half of the smallest positive wire TTL.
        Ok(interval.max(Duration::from_millis(100)))
    }

    /// Refuse, before any I/O, a BBMD no device could register with: one
    /// at port 0, at the unspecified address, or at a group address of a
    /// link whose broadcast IP is `broadcast` on `interface`. One at the
    /// transport's own address is refused once it is bound, when the node
    /// starts.
    pub(super) fn check_bbmd(&self, broadcast: Ipv4Addr, interface: Ipv4Addr) -> Result<(), Error> {
        // Only the IP test is read, so the broadcast's port doesn't matter.
        let groups = Groups {
            broadcast: SocketAddrV4::new(broadcast, 0),
            interface,
        };
        let refusal = if self.bbmd_port == 0 {
            ConfigError::BbmdPort
        } else if self.bbmd_ip.is_unspecified() || groups.contains_ip(self.bbmd_ip) {
            ConfigError::BbmdAddress
        } else {
            return Ok(());
        };
        Err(Error::Encoding(refusal.to_string()))
    }

    /// The node engine's registration, renewed every `interval`
    /// ([`Self::interval`]) as before: an attempt waits for its result at
    /// most that interval or the manual helpers' timeout, whichever is
    /// shorter; the next goes one interval after the last one started,
    /// whatever came of it; broadcasts go at once, and Forwarded-NPDUs are
    /// taken from any sender.
    pub(super) fn engine(&self, interval: Duration) -> Foreign {
        Foreign {
            bbmd: SocketAddrV4::new(self.bbmd_ip, self.bbmd_port),
            ttl: self.ttl,
            renewal: core_duration(interval),
            response_timeout: core_duration(interval.min(BipTransport::BVLC_RESPONSE_TIMEOUT)),
            retry: Retry::AtRenewal,
            hold: 0,
            trust: ForwardedTrust::AnySender,
        }
    }
}
