//! Small, allocation-free time primitives used by the MS/TP core.
//!
//! The core deliberately does not choose a clock implementation.  A platform
//! supplies an [`Instant`] to each event (or implements [`Monotonic`] when it
//! is convenient to ask the platform for the current time).  Values are
//! represented as microseconds from an arbitrary, monotonic epoch.

use core::ops::{Add, AddAssign, Sub, SubAssign};

/// A monotonic timestamp with microsecond resolution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(u64);

impl Instant {
    /// The earliest representable instant.
    pub const ZERO: Self = Self(0);
    /// The latest representable instant.
    pub const MAX: Self = Self(u64::MAX);

    /// Construct an instant from microseconds since the caller's epoch.
    #[must_use]
    pub const fn from_micros(micros: u64) -> Self {
        Self(micros)
    }

    /// Return microseconds since the caller's epoch.
    #[must_use]
    pub const fn as_micros(self) -> u64 {
        self.0
    }

    /// Add a duration, returning `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, duration: Duration) -> Option<Self> {
        match self.0.checked_add(duration.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Add a duration, clamping at [`Instant::MAX`] on overflow.
    #[must_use]
    pub const fn saturating_add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(duration.0))
    }

    /// Subtract a duration, returning `None` on underflow.
    #[must_use]
    pub const fn checked_sub(self, duration: Duration) -> Option<Self> {
        match self.0.checked_sub(duration.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Subtract a duration, clamping at [`Instant::ZERO`] on underflow.
    #[must_use]
    pub const fn saturating_sub(self, duration: Duration) -> Self {
        Self(self.0.saturating_sub(duration.0))
    }

    /// Return the elapsed duration when `self` is no later than `end`.
    #[must_use]
    pub const fn duration_since(self, earlier: Self) -> Option<Duration> {
        match self.0.checked_sub(earlier.0) {
            Some(value) => Some(Duration(value)),
            None => None,
        }
    }

    /// Return the non-negative duration between two instants.
    #[must_use]
    pub const fn saturating_duration_since(self, earlier: Self) -> Duration {
        Duration(self.0.saturating_sub(earlier.0))
    }
}

/// A non-negative duration with microsecond resolution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(u64);

impl Duration {
    /// A zero-length duration.
    pub const ZERO: Self = Self(0);
    /// The largest representable duration.
    pub const MAX: Self = Self(u64::MAX);

    /// Construct a duration from microseconds.
    #[must_use]
    pub const fn from_micros(micros: u64) -> Self {
        Self(micros)
    }

    /// Construct a duration from milliseconds, returning `None` if the
    /// conversion would overflow the microsecond representation.
    #[must_use]
    pub const fn try_from_millis(millis: u64) -> Option<Self> {
        match millis.checked_mul(1_000) {
            Some(micros) => Some(Self(micros)),
            None => None,
        }
    }

    /// Construct a duration from milliseconds, saturating on overflow.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis.saturating_mul(1_000))
    }

    /// Construct a duration from seconds, returning `None` on overflow.
    #[must_use]
    pub const fn try_from_secs(seconds: u64) -> Option<Self> {
        match seconds.checked_mul(1_000_000) {
            Some(micros) => Some(Self(micros)),
            None => None,
        }
    }

    /// Construct a duration from seconds, saturating on overflow.
    #[must_use]
    pub const fn from_secs(seconds: u64) -> Self {
        Self(seconds.saturating_mul(1_000_000))
    }

    /// Return microseconds in this duration.
    #[must_use]
    pub const fn as_micros(self) -> u64 {
        self.0
    }

    /// Return complete milliseconds in this duration.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0 / 1_000
    }

    /// Return complete seconds in this duration.
    #[must_use]
    pub const fn as_secs(self) -> u64 {
        self.0 / 1_000_000
    }

    /// Add a duration, returning `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Add a duration, clamping at [`Duration::MAX`] on overflow.
    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// Subtract a duration, returning `None` on underflow.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Subtract a duration, clamping at [`Duration::ZERO`] on underflow.
    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// The remainder of `self` divided by `divisor`, for a divisor of at
    /// least 1 µs and under [`Self::SHORT_DIVISOR_LIMIT`]; `None` for any
    /// other divisor.
    ///
    /// It divides one octet at a time with 32-bit operations, so a 32-bit
    /// target links no 64-bit division routine for it. The remainder so far
    /// stays below the divisor, so shifting it left by an octet still fits a
    /// `u32`.
    pub(crate) const fn rem_short(self, divisor: Self) -> Option<Self> {
        if divisor.0 == 0 || divisor.0 >= Self::SHORT_DIVISOR_LIMIT.0 {
            return None;
        }
        let divisor = divisor.0 as u32;
        let octets = self.0.to_be_bytes();
        let mut remainder = 0u32;
        let mut index = 0;
        while index < octets.len() {
            remainder = ((remainder << 8) | octets[index] as u32) % divisor;
            index += 1;
        }
        Some(Self(remainder as u64))
    }

    /// The bound on [`Self::rem_short`]'s divisor: 2^24 µs, about 16.8 s.
    pub(crate) const SHORT_DIVISOR_LIMIT: Self = Self(1 << 24);
}

impl Add for Duration {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        self.saturating_add(rhs)
    }
}

impl AddAssign for Duration {
    fn add_assign(&mut self, rhs: Self) {
        *self = self.saturating_add(rhs);
    }
}

impl Sub for Duration {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        self.saturating_sub(rhs)
    }
}

impl SubAssign for Duration {
    fn sub_assign(&mut self, rhs: Self) {
        *self = self.saturating_sub(rhs);
    }
}

impl Add<Duration> for Instant {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self::Output {
        self.saturating_add(rhs)
    }
}

impl AddAssign<Duration> for Instant {
    fn add_assign(&mut self, rhs: Duration) {
        *self = self.saturating_add(rhs);
    }
}

impl Sub<Duration> for Instant {
    type Output = Self;

    fn sub(self, rhs: Duration) -> Self::Output {
        self.saturating_sub(rhs)
    }
}

impl SubAssign<Duration> for Instant {
    fn sub_assign(&mut self, rhs: Duration) {
        *self = self.saturating_sub(rhs);
    }
}

impl Sub for Instant {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Self::Output {
        self.saturating_duration_since(rhs)
    }
}

/// A caller-provided monotonic time source.
pub trait Monotonic {
    /// Return the current timestamp.
    fn now(&self) -> Instant;
}

/// Deterministic clock for host tests and platform bring-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimClock {
    now: Instant,
}

impl SimClock {
    /// Create a clock at the zero epoch.
    #[must_use]
    pub const fn new() -> Self {
        Self { now: Instant::ZERO }
    }

    /// Create a clock at a caller-supplied instant.
    #[must_use]
    pub const fn from_instant(now: Instant) -> Self {
        Self { now }
    }

    /// Return the current simulated instant.
    #[must_use]
    pub const fn now(&self) -> Instant {
        self.now
    }

    /// Set the simulated instant.  Callers are responsible for preserving
    /// monotonicity when using this helper directly.
    pub const fn set(&mut self, now: Instant) {
        self.now = now;
    }

    /// Advance the clock, saturating at [`Instant::MAX`].
    pub const fn advance(&mut self, duration: Duration) {
        self.now = self.now.saturating_add(duration);
    }

    /// Advance by microseconds, saturating at [`Instant::MAX`].
    pub const fn advance_micros(&mut self, micros: u64) {
        self.advance(Duration::from_micros(micros));
    }
}

impl Default for SimClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Monotonic for SimClock {
    fn now(&self) -> Instant {
        self.now()
    }
}

#[cfg(test)]
mod tests {
    use super::{Duration, Instant, Monotonic, SimClock};

    #[test]
    fn duration_conversions_and_saturation_are_deterministic() {
        assert_eq!(Duration::from_millis(2).as_micros(), 2_000);
        assert_eq!(Duration::from_secs(2).as_millis(), 2_000);
        assert_eq!(Duration::MAX.checked_add(Duration::from_micros(1)), None);
        assert_eq!(Duration::MAX + Duration::from_micros(1), Duration::MAX);
        assert_eq!(Duration::ZERO - Duration::from_micros(1), Duration::ZERO);
    }

    #[test]
    fn instant_checked_and_saturating_arithmetic() {
        let now = Instant::from_micros(10);
        assert_eq!(
            now.checked_add(Duration::from_micros(2)),
            Some(Instant::from_micros(12))
        );
        assert_eq!(now.checked_sub(Duration::from_micros(11)), None);
        assert_eq!(
            now.saturating_duration_since(Instant::from_micros(12)),
            Duration::ZERO
        );
        assert_eq!(now - Instant::from_micros(4), Duration::from_micros(6));
    }

    #[test]
    fn short_remainders_match_the_64_bit_ones() {
        let limit = Duration::SHORT_DIVISOR_LIMIT.as_micros();
        for divisor in [1, 2, 10_000, 1_280_000, limit - 1] {
            for value in [0, 1, divisor - 1, divisor, u64::MAX, u64::MAX - 7, 1 << 40] {
                assert_eq!(
                    Duration::from_micros(value).rem_short(Duration::from_micros(divisor)),
                    Some(Duration::from_micros(value % divisor)),
                    "{value} % {divisor}"
                );
            }
        }
        for divisor in [0, limit, u64::MAX] {
            assert_eq!(
                Duration::from_micros(5).rem_short(Duration::from_micros(divisor)),
                None
            );
        }
    }

    proptest::proptest! {
        #[test]
        fn short_remainders_match_for_any_value(
            value in proptest::prelude::any::<u64>(),
            divisor in 1u64..(1 << 24),
        ) {
            proptest::prop_assert_eq!(
                Duration::from_micros(value).rem_short(Duration::from_micros(divisor)),
                Some(Duration::from_micros(value % divisor))
            );
        }
    }

    #[test]
    fn simulated_clock_implements_monotonic() {
        let mut clock = SimClock::new();
        assert_eq!(Monotonic::now(&clock), Instant::ZERO);
        clock.advance(Duration::from_millis(3));
        assert_eq!(clock.now().as_micros(), 3_000);
        clock.advance_micros(7);
        assert_eq!(clock.now(), Instant::from_micros(3_007));
    }
}
