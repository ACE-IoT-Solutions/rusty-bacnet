//! Conversions to and from `core::time::Duration`.

use crate::divide::div_rem;
use crate::{Duration, NANOS_PER_SEC};

impl From<core::time::Duration> for Duration {
    /// The same length, saturating at [`Duration::MAX`]: a
    /// `core::time::Duration` reaches about 584 billion years.
    fn from(duration: core::time::Duration) -> Self {
        let nanos = duration
            .as_secs()
            .checked_mul(NANOS_PER_SEC)
            .and_then(|nanos| nanos.checked_add(u64::from(duration.subsec_nanos())));
        match nanos {
            Some(nanos) => Self::from_nanos(nanos),
            None => Self::MAX,
        }
    }
}

impl From<Duration> for core::time::Duration {
    /// The same length, exactly. Splitting it into seconds and nanoseconds
    /// takes no 64-bit division routine (see the
    /// [crate docs](crate#no-64-bit-division)).
    fn from(duration: Duration) -> Self {
        let (secs, nanos) = div_rem(duration.as_nanos(), NANOS_PER_SEC);
        // The remainder is under one second, so it fits a u32.
        Self::new(secs, nanos as u32)
    }
}
