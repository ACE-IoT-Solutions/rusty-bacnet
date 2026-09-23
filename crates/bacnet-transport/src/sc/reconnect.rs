use bacnet_types::error::Error;

/// Configuration for SC transport reconnection with exponential backoff.
#[derive(Debug, Clone)]
pub struct ScReconnectConfig {
    /// Initial delay before first reconnect attempt (ms).
    pub initial_delay_ms: u64,
    /// Maximum delay between reconnect attempts (ms).
    pub max_delay_ms: u64,
    /// Maximum reconnect attempts on the active hub after a disconnect.
    /// Zero skips these retries, not the initial connection, eligible failover,
    /// or primary restoration while connected to failover.
    pub max_retries: u32,
    /// Retry without a count limit. Retry-forbidden protocol outcomes still stop.
    pub retry_forever: bool,
}

impl Default for ScReconnectConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 10_000,
            max_delay_ms: 600_000,
            max_retries: 10,
            retry_forever: false,
        }
    }
}

impl ScReconnectConfig {
    /// Construct an unbounded reconnect policy with capped exponential backoff.
    pub fn unbounded(initial_delay_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            initial_delay_ms,
            max_delay_ms,
            max_retries: 0,
            retry_forever: true,
        }
    }
    /// Check that delays are nonzero and the initial delay does not exceed the maximum.
    ///
    /// This defensive guard applies even when `max_retries` is zero. It does not
    /// impose a production minimum delay, timeout cap, or retry-count policy;
    /// acceptance does not establish deployment safety or Annex AB.6.1 conformance.
    /// Call this before dialing if supplying a socket to a raw SC transport.
    pub fn validate(&self) -> Result<(), Error> {
        if self.initial_delay_ms == 0 {
            return Err(Error::OutOfRange(
                "BACnet/SC reconnect initial_delay_ms must be greater than zero".into(),
            ));
        }
        if self.max_delay_ms == 0 {
            return Err(Error::OutOfRange(
                "BACnet/SC reconnect max_delay_ms must be greater than zero".into(),
            ));
        }
        if self.initial_delay_ms > self.max_delay_ms {
            return Err(Error::OutOfRange(format!(
                "BACnet/SC reconnect initial_delay_ms must not exceed max_delay_ms, \
                 got initial_delay_ms={} max_delay_ms={}",
                self.initial_delay_ms, self.max_delay_ms
            )));
        }
        Ok(())
    }
}

impl super::ScConnection {
    pub(super) fn reset_for_connect_retry(&mut self) {
        *self = Self::new(self.local_vmac, self.device_uuid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_validation_rejects_zero_and_inverted_delays_for_all_retry_counts() {
        for max_retries in [0, 1, 10, u32::MAX] {
            for (initial_delay_ms, max_delay_ms, field) in [
                (0, 1, "initial_delay_ms"),
                (1, 0, "max_delay_ms"),
                (0, 0, "initial_delay_ms"),
                (2, 1, "initial_delay_ms"),
                (u64::MAX, u64::MAX - 1, "initial_delay_ms"),
            ] {
                let config = ScReconnectConfig {
                    initial_delay_ms,
                    max_delay_ms,
                    max_retries,
                    retry_forever: false,
                };
                assert!(
                    matches!(config.validate(), Err(Error::OutOfRange(message))
                        if message.contains("reconnect") && message.contains(field)),
                    "{config:?}"
                );
            }
        }
    }

    #[test]
    fn reconnect_validation_accepts_positive_ordered_delays_without_timer_or_retry_caps() {
        let default = ScReconnectConfig::default();
        assert_eq!(default.initial_delay_ms, 10_000);
        assert_eq!(default.max_delay_ms, 600_000);
        assert_eq!(default.max_retries, 10);
        assert!(!default.retry_forever);
        default.validate().unwrap();

        // Validate only: accepted extreme values are not safe timer/deployment promises.
        for max_retries in [0, 1, 10, u32::MAX] {
            for (initial_delay_ms, max_delay_ms) in [
                (1, 1),
                (1, 2),
                (10_000, 600_000),
                (600_001, 600_001),
                (1, u64::MAX),
                (u64::MAX - 1, u64::MAX),
                (u64::MAX, u64::MAX),
            ] {
                ScReconnectConfig {
                    initial_delay_ms,
                    max_delay_ms,
                    max_retries,
                    retry_forever: false,
                }
                .validate()
                .unwrap();
            }
        }
    }

    #[test]
    fn unbounded_helper_preserves_zero_as_a_bounded_only_semantic() {
        let config = ScReconnectConfig::unbounded(25, 1_000);
        assert!(config.retry_forever);
        assert_eq!(config.max_retries, 0);
        config.validate().unwrap();

        let bounded = ScReconnectConfig {
            max_retries: 0,
            ..ScReconnectConfig::default()
        };
        assert!(!bounded.retry_forever);
    }
}
