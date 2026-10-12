//! Time the caller supplies: a monotonic [`Instant`] and a [`Duration`].
//!
//! The crate reads no clock. Each call that depends on time takes `now`, an
//! [`Instant`] on whatever monotonic timeline the caller keeps, counted in
//! nanoseconds from an epoch the caller chooses. Nanoseconds let a host map
//! its own monotonic clock onto the timeline exactly; firmware with a
//! millisecond tick uses [`Instant::from_millis`].
//!
//! Nothing here divides a 64-bit value, so a 32-bit target links no 64-bit
//! division routine for it.

use core::ops::{Add, AddAssign, Sub, SubAssign};

/// Nanoseconds in one second.
const NANOS_PER_SEC: u64 = 1_000_000_000;

/// A point on the caller's monotonic timeline, in nanoseconds since the
/// caller's epoch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(u64);

impl Instant {
    /// The epoch.
    pub const ZERO: Self = Self(0);
    /// The latest representable instant.
    pub const MAX: Self = Self(u64::MAX);

    /// The instant `nanos` nanoseconds after the epoch.
    #[must_use]
    pub const fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    /// The instant `micros` microseconds after the epoch, saturating at
    /// [`Instant::MAX`].
    #[must_use]
    pub const fn from_micros(micros: u64) -> Self {
        Self(micros.saturating_mul(1_000))
    }

    /// The instant `millis` milliseconds after the epoch, saturating at
    /// [`Instant::MAX`].
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis.saturating_mul(1_000_000))
    }

    /// The instant `secs` seconds after the epoch, saturating at
    /// [`Instant::MAX`].
    #[must_use]
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs.saturating_mul(NANOS_PER_SEC))
    }

    /// Nanoseconds since the epoch.
    #[must_use]
    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// `self + duration`, or `None` past [`Instant::MAX`].
    #[must_use]
    pub const fn checked_add(self, duration: Duration) -> Option<Self> {
        match self.0.checked_add(duration.0) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// `self + duration`, clamped at [`Instant::MAX`].
    #[must_use]
    pub const fn saturating_add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(duration.0))
    }

    /// `self - duration`, or `None` before the epoch.
    #[must_use]
    pub const fn checked_sub(self, duration: Duration) -> Option<Self> {
        match self.0.checked_sub(duration.0) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// `self - duration`, clamped at the epoch.
    #[must_use]
    pub const fn saturating_sub(self, duration: Duration) -> Self {
        Self(self.0.saturating_sub(duration.0))
    }

    /// The time from `earlier` to `self`, or `None` when `earlier` is later.
    #[must_use]
    pub const fn checked_duration_since(self, earlier: Self) -> Option<Duration> {
        match self.0.checked_sub(earlier.0) {
            Some(nanos) => Some(Duration(nanos)),
            None => None,
        }
    }

    /// The time from `earlier` to `self`, or zero when `earlier` is later.
    #[must_use]
    pub const fn saturating_duration_since(self, earlier: Self) -> Duration {
        Duration(self.0.saturating_sub(earlier.0))
    }
}

/// A length of time in nanoseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(u64);

impl Duration {
    /// No time.
    pub const ZERO: Self = Self(0);
    /// The longest representable duration, about 584 years.
    pub const MAX: Self = Self(u64::MAX);

    /// `nanos` nanoseconds.
    #[must_use]
    pub const fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    /// `micros` microseconds, saturating at [`Duration::MAX`].
    #[must_use]
    pub const fn from_micros(micros: u64) -> Self {
        Self(micros.saturating_mul(1_000))
    }

    /// `millis` milliseconds, saturating at [`Duration::MAX`].
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis.saturating_mul(1_000_000))
    }

    /// `secs` seconds, saturating at [`Duration::MAX`].
    #[must_use]
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs.saturating_mul(NANOS_PER_SEC))
    }

    /// The length in nanoseconds.
    #[must_use]
    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// `self + other`, or `None` past [`Duration::MAX`].
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// `self + other`, clamped at [`Duration::MAX`].
    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// `self - other`, or `None` below zero.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// `self - other`, clamped at zero.
    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// Half of `self`, rounded down: a shift, so no division routine.
    #[must_use]
    pub const fn half(self) -> Self {
        Self(self.0 >> 1)
    }

    /// Twice `self`, clamped at [`Duration::MAX`]: a shift, so no
    /// multiplication routine.
    #[must_use]
    pub const fn saturating_double(self) -> Self {
        if self.0 > u64::MAX >> 1 {
            Self::MAX
        } else {
            Self(self.0 << 1)
        }
    }

    /// The shorter of two durations, in a `const` context.
    pub(crate) const fn min_const(self, other: Self) -> Self {
        if self.0 <= other.0 {
            self
        } else {
            other
        }
    }

    /// The longer of two durations, in a `const` context.
    pub(crate) const fn max_const(self, other: Self) -> Self {
        if self.0 >= other.0 {
            self
        } else {
            other
        }
    }

    /// Whole seconds in `self`, for a duration under 2^17 seconds (about 36
    /// hours); `None` for a longer one.
    ///
    /// Binary long division by shifted copies of one second: compares and
    /// subtractions only, so no 64-bit division routine is linked.
    pub(crate) const fn whole_secs_short(self) -> Option<u32> {
        /// The quotient's top bit: 2^16 seconds.
        const TOP: u32 = 16;
        if self.0 >= NANOS_PER_SEC << (TOP + 1) {
            return None;
        }
        let mut rest = self.0;
        let mut step = NANOS_PER_SEC << TOP;
        let mut bit = 1u32 << TOP;
        let mut secs = 0;
        while bit != 0 {
            if rest >= step {
                rest -= step;
                secs |= bit;
            }
            step >>= 1;
            bit >>= 1;
        }
        Some(secs)
    }
}

impl Add for Duration {
    type Output = Self;

    /// Saturating, as [`Duration::saturating_add`].
    fn add(self, rhs: Self) -> Self {
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

    /// Saturating, as [`Duration::saturating_sub`].
    fn sub(self, rhs: Self) -> Self {
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

    /// Saturating, as [`Instant::saturating_add`].
    fn add(self, rhs: Duration) -> Self {
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

    /// Saturating, as [`Instant::saturating_sub`].
    fn sub(self, rhs: Duration) -> Self {
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

    /// Saturating, as [`Instant::saturating_duration_since`].
    fn sub(self, rhs: Self) -> Duration {
        self.saturating_duration_since(rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::{Duration, Instant, NANOS_PER_SEC};

    #[test]
    fn constructors_scale_and_saturate() {
        assert_eq!(Instant::from_micros(3).as_nanos(), 3_000);
        assert_eq!(Instant::from_millis(3).as_nanos(), 3_000_000);
        assert_eq!(Instant::from_secs(3).as_nanos(), 3 * NANOS_PER_SEC);
        assert_eq!(Instant::from_secs(u64::MAX), Instant::MAX);
        assert_eq!(Duration::from_micros(2).as_nanos(), 2_000);
        assert_eq!(Duration::from_millis(2).as_nanos(), 2_000_000);
        assert_eq!(
            Duration::from_secs(2),
            Duration::from_nanos(2 * NANOS_PER_SEC)
        );
        assert_eq!(Duration::from_millis(u64::MAX), Duration::MAX);
        assert_eq!(Duration::from_micros(u64::MAX), Duration::MAX);
        assert_eq!(Instant::from_micros(u64::MAX), Instant::MAX);
        assert_eq!(Instant::from_millis(u64::MAX), Instant::MAX);
    }

    #[test]
    fn instant_arithmetic_is_checked_or_saturating() {
        let t = Instant::from_nanos(10);
        let d = Duration::from_nanos(4);
        assert_eq!(t.checked_add(d), Some(Instant::from_nanos(14)));
        assert_eq!(Instant::MAX.checked_add(d), None);
        assert_eq!(Instant::MAX.saturating_add(d), Instant::MAX);
        assert_eq!(t.checked_sub(d), Some(Instant::from_nanos(6)));
        assert_eq!(Instant::ZERO.checked_sub(d), None);
        assert_eq!(Instant::ZERO.saturating_sub(d), Instant::ZERO);
        assert_eq!(t + d, Instant::from_nanos(14));
        assert_eq!(t - d, Instant::from_nanos(6));
        let mut u = t;
        u += d;
        u -= Duration::from_nanos(1);
        assert_eq!(u, Instant::from_nanos(13));
        assert_eq!(
            t.checked_duration_since(Instant::from_nanos(4)),
            Some(Duration::from_nanos(6))
        );
        assert_eq!(t.checked_duration_since(Instant::from_nanos(11)), None);
        assert_eq!(
            t.saturating_duration_since(Instant::from_nanos(11)),
            Duration::ZERO
        );
        assert_eq!(t - Instant::from_nanos(4), Duration::from_nanos(6));
    }

    #[test]
    fn duration_arithmetic_is_checked_or_saturating() {
        let a = Duration::from_nanos(7);
        let b = Duration::from_nanos(3);
        assert_eq!(a.checked_add(b), Some(Duration::from_nanos(10)));
        assert_eq!(Duration::MAX.checked_add(b), None);
        assert_eq!(Duration::MAX + b, Duration::MAX);
        assert_eq!(a.checked_sub(b), Some(Duration::from_nanos(4)));
        assert_eq!(b.checked_sub(a), None);
        assert_eq!(b - a, Duration::ZERO);
        let mut c = a;
        c += b;
        c -= Duration::from_nanos(1);
        assert_eq!(c, Duration::from_nanos(9));
    }

    #[test]
    fn halving_and_doubling_are_shifts_that_saturate() {
        assert_eq!(Duration::from_nanos(7).half(), Duration::from_nanos(3));
        assert_eq!(Duration::ZERO.half(), Duration::ZERO);
        assert_eq!(Duration::from_secs(30).half(), Duration::from_secs(15));
        assert_eq!(
            Duration::from_nanos(3).saturating_double(),
            Duration::from_nanos(6)
        );
        assert_eq!(
            Duration::from_nanos(u64::MAX >> 1).saturating_double(),
            Duration::from_nanos(u64::MAX - 1)
        );
        assert_eq!(
            Duration::from_nanos((u64::MAX >> 1) + 1).saturating_double(),
            Duration::MAX
        );
        assert_eq!(Duration::MAX.saturating_double(), Duration::MAX);
        let (a, b) = (Duration::from_nanos(2), Duration::from_nanos(5));
        assert_eq!(a.min_const(b), a);
        assert_eq!(b.min_const(a), a);
        assert_eq!(a.max_const(b), b);
        assert_eq!(b.max_const(a), b);
    }

    #[test]
    fn whole_seconds_truncate_and_stop_at_the_bound() {
        assert_eq!(Duration::ZERO.whole_secs_short(), Some(0));
        assert_eq!(
            Duration::from_nanos(NANOS_PER_SEC - 1).whole_secs_short(),
            Some(0)
        );
        assert_eq!(Duration::from_secs(1).whole_secs_short(), Some(1));
        assert_eq!(
            Duration::from_nanos(65_565 * NANOS_PER_SEC + 1).whole_secs_short(),
            Some(65_565)
        );
        let limit = NANOS_PER_SEC << 17;
        assert_eq!(
            Duration::from_nanos(limit - 1).whole_secs_short(),
            Some((1 << 17) - 1)
        );
        assert_eq!(Duration::from_nanos(limit).whole_secs_short(), None);
        assert_eq!(Duration::MAX.whole_secs_short(), None);
    }

    proptest::proptest! {
        #[test]
        fn whole_seconds_match_division(nanos in 0u64..(NANOS_PER_SEC << 17)) {
            let expected = u32::try_from(nanos / NANOS_PER_SEC).unwrap();
            proptest::prop_assert_eq!(Duration::from_nanos(nanos).whole_secs_short(), Some(expected));
        }
    }
}
