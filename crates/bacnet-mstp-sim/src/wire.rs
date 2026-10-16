//! Bit timing on the simulated wire.

use bacnet_mstp_core::{Duration, Instant};

/// Bit times one octet occupies: a start bit, eight data bits and a stop bit,
/// with no parity (Clause 9.2.3).
pub const BITS_PER_OCTET: u64 = 10;

/// Converts bit and octet counts to virtual time at one baud rate.
///
/// Octet boundaries are measured from the start of a transmission and rounded
/// up to a whole microsecond each, so rounding never accumulates over a long
/// frame: octet `i` of a transmission that starts at `t` occupies
/// `t + boundary(i) .. t + boundary(i + 1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireTiming {
    baud: u32,
}

impl WireTiming {
    /// Timing for `baud` bits per second.
    ///
    /// # Panics
    ///
    /// If `baud` is zero.
    #[must_use]
    pub fn new(baud: u32) -> Self {
        assert!(baud > 0, "the simulated line needs a non-zero baud rate");
        Self { baud }
    }

    /// The baud rate.
    #[must_use]
    pub const fn baud(self) -> u32 {
        self.baud
    }

    /// `bits` bit times, rounded up to a whole microsecond.
    #[must_use]
    pub fn bits(self, bits: u64) -> Duration {
        Duration::from_micros((bits * 1_000_000).div_ceil(u64::from(self.baud)))
    }

    /// Time from the start of a transmission to the start of its octet
    /// `octets` (equally, to the end of the octet before it).
    #[must_use]
    pub fn boundary(self, octets: usize) -> Duration {
        let octets = u64::try_from(octets).unwrap_or(u64::MAX);
        self.bits(octets.saturating_mul(BITS_PER_OCTET))
    }

    /// One octet time, rounded up.
    #[must_use]
    pub fn octet(self) -> Duration {
        self.boundary(1)
    }

    /// Largest whole-nanosecond hold after the final stop bit's end.
    ///
    /// 135-2020 9.2.3 counts a maximum 15 bit times from the stop bit's
    /// beginning; 9.5.3 counts from its end. This model uses their conservative
    /// intersection: at most 14 bit times after the end. The upper bound is
    /// rounded down, unlike character boundaries, so it cannot be widened.
    #[must_use]
    pub fn max_post_drive(self) -> Duration {
        Duration::from_nanos(14_000_000_000 / u64::from(self.baud))
    }

    pub(crate) fn validate_post_drive(self, hold: Duration) {
        assert!(
            u128::from(hold.as_nanos()) * u128::from(self.baud) <= 14_000_000_000,
            "post-drive hold exceeds 14 bit times after the final stop bit"
        );
    }

    /// When a transmission of `octets` octets that starts at `start` ends.
    #[must_use]
    pub fn end_of(self, start: Instant, octets: usize) -> Instant {
        start + self.boundary(octets)
    }
}

#[cfg(test)]
mod tests {
    use super::WireTiming;
    use bacnet_mstp_core::{Duration, Instant};

    #[test]
    fn boundaries_round_up_without_drift() {
        let timing = WireTiming::new(9_600);
        // 10 bits at 9600 baud is 1041.67 us.
        assert_eq!(timing.octet(), Duration::from_micros(1_042));
        // Three octets are 3125 us exactly; summing rounded octets would give 3126.
        assert_eq!(timing.boundary(3), Duration::from_micros(3_125));
        assert_eq!(timing.boundary(8), Duration::from_micros(8_334));
        assert_eq!(
            timing.end_of(Instant::from_micros(10), 3),
            Instant::from_micros(3_135)
        );
        // 40 bit times, Tturnaround's floor at 38400 baud.
        assert_eq!(
            WireTiming::new(38_400).bits(40),
            Duration::from_micros(1_042)
        );
    }

    #[test]
    fn post_drive_limit_rounds_down_and_rejects_the_next_nanosecond() {
        for (baud, max_ns) in [(10_000, 1_400_000), (115_200, 121_527), (9_600, 1_458_333)] {
            let timing = WireTiming::new(baud);
            assert_eq!(timing.max_post_drive(), Duration::from_nanos(max_ns));
            timing.validate_post_drive(Duration::ZERO);
            timing.validate_post_drive(Duration::from_nanos(max_ns));
            assert!(std::panic::catch_unwind(|| {
                timing.validate_post_drive(Duration::from_nanos(max_ns + 1));
            })
            .is_err());
            assert!(std::panic::catch_unwind(|| {
                timing.validate_post_drive(timing.bits(15));
            })
            .is_err());
        }
        // Rounding the 14-bit upper bound up to a character's microsecond
        // granularity would admit 122 us at 115200, exceeding the bound.
        assert!(std::panic::catch_unwind(|| {
            WireTiming::new(115_200).validate_post_drive(Duration::from_micros(122));
        })
        .is_err());
    }
}
