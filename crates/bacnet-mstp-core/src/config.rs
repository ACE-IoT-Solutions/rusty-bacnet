//! Configuration and Clause 9 timing validation for the MS/TP master core.

use bacnet_mstp_codec::{
    MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA, MAX_STANDARD_FRAME_LENGTH,
    MAX_STANDARD_MPDU_DATA,
};
use bacnet_time::Duration;

const MAX_MASTER_ADDRESS: u8 = 127;
const DEFAULT_BAUD_RATE: u32 = 9_600;
const MAX_FRAME_ABORT: Duration = Duration::from_millis(100);
/// The shortest T_reply_timeout a station may use (Clause 9.5.3). No requester
/// stops waiting for a reply sooner, so it bounds how late a reply may start.
pub(crate) const MIN_REPLY_TIMEOUT: Duration = Duration::from_millis(255);
/// The shortest T_usage_timeout a station may use (Clause 9.5.3). No poller
/// gives up on a Reply To Poll For Master sooner.
pub(crate) const MIN_USAGE_TIMEOUT: Duration = Duration::from_millis(20);
/// Default for [`MstpCoreConfig::reply_transmit_margin`], the value the host
/// MS/TP transport reserves.
const DEFAULT_REPLY_TRANSMIT_MARGIN: Duration = Duration::from_millis(25);
/// Default for [`MstpCoreConfig::transmit_watchdog`] where it clears the
/// longest frame.
const DEFAULT_TRANSMIT_WATCHDOG: Duration = Duration::from_secs(1);
/// Kept in [`MstpCoreConfig::minimum_transmit_watchdog`] beyond the longest
/// frame and T_postdrive, for the adapter to report the completion.
const WATCHDOG_COMPLETION_MARGIN: Duration = Duration::from_millis(1);

/// A configuration value that violates a protocol or local safety bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// A baud rate of zero cannot be used to derive bit-time limits.
    ZeroBaudRate,
    /// A derived timing calculation overflowed its arithmetic.
    TimingOverflow,
    /// Clause 9 fixes Npoll at 50 token uses.
    InvalidPollCount {
        /// Supplied poll count.
        value: u8,
    },
    /// Clause 9 fixes Nretry_token at one retry.
    InvalidTokenRetryCount {
        /// Supplied token retry count.
        value: u8,
    },
    /// Clause 9 fixes Nmin_octets at four octets.
    InvalidMinimumOctets {
        /// Supplied minimum idle-octet count.
        value: u8,
    },
    /// Max_Master is a seven-bit station address.
    MaxMasterOutOfRange {
        /// Supplied Max_Master value.
        value: u8,
    },
    /// This station cannot be beyond Max_Master.
    StationExceedsMaxMaster {
        /// Local station address.
        station: u8,
        /// Configured Max_Master value.
        max_master: u8,
    },
    /// Max_Info_Frames must be in the inclusive 1..=255 range.
    MaxInfoFramesOutOfRange {
        /// Supplied Max_Info_Frames value.
        value: u8,
    },
    /// T_frame_abort is outside the 60-bit-time through 100 ms range.
    FrameAbortOutOfRange {
        /// Supplied frame-abort duration.
        value: Duration,
        /// Minimum permitted duration.
        minimum: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// T_frame_gap is longer than 20 bit times.
    FrameGapTooLong {
        /// Supplied frame-gap duration.
        value: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// T_postdrive is longer than 15 bit times.
    PostdriveTooLong {
        /// Supplied postdrive duration.
        value: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// T_no_token is the fixed 500 ms lost-token deadline.
    InvalidNoToken {
        /// Supplied no-token duration.
        value: Duration,
    },
    /// T_reply_delay cannot exceed 250 ms.
    ReplyDelayTooLong {
        /// Supplied reply-delay duration.
        value: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// The reply transmit margin must leave part of T_reply_delay for the
    /// application to answer.
    ReplyMarginTooLong {
        /// Supplied margin.
        margin: Duration,
        /// Configured T_reply_delay.
        reply_delay: Duration,
    },
    /// T_reply_timeout must be between 255 and 300 ms.
    ReplyTimeoutOutOfRange {
        /// Supplied reply-timeout duration.
        value: Duration,
        /// Minimum permitted duration.
        minimum: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// T_slot is the ten-millisecond maintenance slot.
    InvalidSlot {
        /// Supplied maintenance-slot duration.
        value: Duration,
    },
    /// T_turnaround must cover at least 40 bit times.
    TurnaroundTooShort {
        /// Supplied turnaround duration.
        value: Duration,
        /// Minimum permitted duration.
        minimum: Duration,
    },
    /// T_turnaround cannot exceed the maximum T_usage_delay response window.
    TurnaroundExceedsUsageDelay {
        /// Supplied turnaround duration.
        turnaround: Duration,
        /// Configured usage-delay duration.
        usage_delay: Duration,
    },
    /// T_usage_delay cannot exceed 15 ms.
    UsageDelayTooLong {
        /// Supplied usage-delay duration.
        value: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// T_usage_timeout must be between 20 and 35 ms.
    UsageTimeoutOutOfRange {
        /// Supplied usage-timeout duration.
        value: Duration,
        /// Minimum permitted duration.
        minimum: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// A local queue age or watchdog of zero would disable the safety bound.
    ZeroLocalDuration {
        /// Local duration setting that was zero.
        field: LocalDurationField,
    },
    /// A local scheduling ratio or starvation limit of zero is invalid.
    ZeroLocalLimit {
        /// Local count or ratio setting that was zero.
        field: LocalLimitField,
    },
    /// Multiple-frame ratio is a percentage and cannot exceed 100.
    MultipleFrameRatioOutOfRange {
        /// Supplied percentage value.
        value: u8,
    },
    /// The transmit watchdog would abort the longest frame this
    /// configuration sends before the driver could go off: a standard frame
    /// of 511 octets takes 1.06 s at 4800 baud, and an extended one of 1516
    /// octets 1.58 s at 9600. See
    /// [`MstpCoreConfig::minimum_transmit_watchdog`]; the baud-rate
    /// constructors and [`MstpCoreConfig::with_extended_frames`] set one
    /// that clears it.
    TransmitWatchdogTooShort {
        /// Supplied watchdog.
        value: Duration,
        /// The shortest watchdog accepted,
        /// [`MstpCoreConfig::minimum_transmit_watchdog`].
        minimum: Duration,
    },
    /// [`MstpCoreConfig::extended_frames`] is on, but a buffer the core was
    /// given can't hold the largest extended frame or NPDU. Only
    /// [`MasterCore::new`](crate::MasterCore::new) reports it, since only it
    /// sees the buffers.
    ExtendedStorageTooSmall {
        /// The buffer that is too small.
        buffer: CoreBuffer,
        /// Its size in octets.
        length: usize,
        /// The size extended frames need: the longest extended frame on the
        /// wire for the receive storage and the transmit frame, the longest
        /// NPDU one carries for a delivery slot and a queue slot.
        minimum: usize,
    },
}

/// The buffers [`ConfigError::ExtendedStorageTooSmall`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreBuffer {
    /// The receive storage, which must hold [`MAX_EXTENDED_FRAME_LENGTH`]
    /// octets.
    ReceiveStorage,
    /// The [`ActionBuffers`](crate::ActionBuffers) frame buffer, its `FRAME`,
    /// which must hold as much.
    TransmitFrame,
    /// Each [`ActionBuffers`](crate::ActionBuffers) delivery slot, its
    /// `NPDU`, which must hold
    /// [`MAX_EXTENDED_MPDU_DATA`]
    /// octets.
    DeliverySlot,
    /// Each queue slot, as
    /// [`QueueStorage::max_npdu_length`](crate::QueueStorage::max_npdu_length)
    /// reports it, which must hold as much.
    QueueSlot,
}

/// Names of local duration settings used by [`ConfigError::ZeroLocalDuration`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalDurationField {
    /// [`MstpCoreConfig::queue_max_age`].
    QueueMaxAge,
    /// [`MstpCoreConfig::transmit_watchdog`].
    TransmitWatchdog,
}

/// Names of local count/ratio settings used by [`ConfigError::ZeroLocalLimit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalLimitField {
    /// [`MstpCoreConfig::broadcast_unicast_ratio`].
    BroadcastUnicastRatio,
    /// [`MstpCoreConfig::multiple_frame_ratio`].
    MultipleFrameRatio,
    /// [`MstpCoreConfig::starvation_limit`].
    StarvationLimit,
}

/// All timing, addressing, and bounded-queue parameters needed by an MS/TP
/// master.
///
/// Durations are `bacnet-time` [`Duration`]s, in nanoseconds. Values whose
/// limits are stated in bit times are still stored as absolute durations so a
/// core instance can keep absolute deadlines without retaining a platform
/// clock or baud-rate conversion helper. The bit-time values the constructors
/// derive from the baud rate, and the bounds [`Self::validate`] checks them
/// against, are whole microseconds (see [`Self::octet_time`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MstpCoreConfig {
    /// This station's MAC address.
    pub station: u8,
    /// Max_Master, inclusive.
    pub max_master: u8,
    /// Max_Info_Frames, the number of information frames per token use.
    pub max_info_frames: u8,
    /// Serial baud rate used to validate bit-time-derived limits.
    pub baud_rate: u32,

    /// Npoll: token uses between maintenance polls.
    pub n_poll: u8,
    /// Nretry_token: token retries before finding a new successor.
    pub n_retry_token: u8,
    /// Nmin_octets: minimum idle octets before a new transmission.
    pub n_min_octets: u8,

    /// T_frame_abort.
    pub t_frame_abort: Duration,
    /// T_frame_gap.
    pub t_frame_gap: Duration,
    /// T_no_token.
    pub t_no_token: Duration,
    /// T_postdrive.
    pub t_postdrive: Duration,
    /// T_reply_delay: the longest a reply to a data-expecting-reply frame may
    /// take to start.
    pub t_reply_delay: Duration,
    /// T_reply_timeout.
    pub t_reply_timeout: Duration,
    /// T_slot.
    pub t_slot: Duration,
    /// T_turnaround.
    pub t_turnaround: Duration,
    /// T_usage_delay.
    pub t_usage_delay: Duration,
    /// T_usage_timeout.
    pub t_usage_timeout: Duration,

    /// Time kept back from T_reply_delay for the platform to get a reply onto
    /// the wire.
    ///
    /// The core waits for the application's answer until T_reply_delay less
    /// this margin, then sends Reply Postponed itself, so that adapter and
    /// UART latency still lets the frame start within T_reply_delay. It must
    /// be shorter than T_reply_delay. Default 25 ms.
    pub reply_transmit_margin: Duration,
    /// Maximum time an item may remain in a local transmit queue.
    pub queue_max_age: Duration,
    /// Number of broadcast frames scheduled per unicast frame while both
    /// classes remain eligible.
    pub broadcast_unicast_ratio: u8,
    /// Percentage of `max_info_frames` usable during one token hold.
    ///
    /// The effective limit is rounded up and is always at least one. A value
    /// of 100 leaves Clause 9 `Max_Info_Frames` unchanged.
    pub multiple_frame_ratio: u8,
    /// Number of service opportunities after which an item is promoted.
    pub starvation_limit: u8,
    /// Maximum time allowed for one platform transmit operation, from its
    /// confirmed start to its completion.
    ///
    /// It must clear [`Self::minimum_transmit_watchdog`], the longest
    /// frame's time on the wire plus T_postdrive, or validation refuses it
    /// with [`ConfigError::TransmitWatchdogTooShort`]. The baud-rate
    /// constructors give one second, or, where the longest standard frame
    /// would not fit in that (below about 5.1 kbaud), the longest frame's
    /// time plus one second.
    pub transmit_watchdog: Duration,

    /// Take and send extended frames, Types 32 and 33, which carry NPDUs of
    /// 502 to 1497 octets COBS-encoded (Clauses 9.3.9, 9.3.10). Off by
    /// default.
    ///
    /// Clause 9.3 requires COBS-encoded frames of a routing node and leaves
    /// them optional for any other node, so a router turns this on for
    /// every MS/TP port it routes and a device leaves it off unless it wants
    /// the larger NPDUs.
    ///
    /// - **Off**, the core is a node without COBS support. It refuses an
    ///   NPDU over 501 octets, and an extended frame it hears counts as an
    ///   invalid frame when it is for this station or broadcast.
    /// - **On**, a Type 32 for this station is a data request like a Type 5,
    ///   and a Type 33 data like a Type 6, a reply to this station's request
    ///   included; broadcasts of either go up as for Types 5 and 6. A
    ///   reserved COBS type (34-127) is checked and, with a good CRC-32K,
    ///   taken as a valid frame of a type the core doesn't know (Clause
    ///   9.5.4.9). An NPDU of 502 to 1497 octets, queued or as a reply, goes
    ///   out as Type 32 when it expects a reply and Type 33 otherwise.
    ///   [`MasterCore::new`](crate::MasterCore::new) then needs buffers
    ///   sized for extended frames, and refuses smaller ones with
    ///   [`ConfigError::ExtendedStorageTooSmall`]. The transmit watchdog
    ///   must clear the longest extended frame
    ///   ([`ConfigError::TransmitWatchdogTooShort`]);
    ///   [`Self::with_extended_frames`] sets both.
    pub extended_frames: bool,
}

impl Default for MstpCoreConfig {
    fn default() -> Self {
        Self::for_baud_rate_unchecked(DEFAULT_BAUD_RATE)
    }
}

impl MstpCoreConfig {
    /// Construct standard-safe settings for a baud rate.
    pub fn for_baud_rate(baud_rate: u32) -> Result<Self, ConfigError> {
        if baud_rate == 0 {
            return Err(ConfigError::ZeroBaudRate);
        }
        let config = Self::for_baud_rate_unchecked(baud_rate);
        config.validate()?;
        Ok(config)
    }

    /// Return the standard 9600-baud configuration.
    #[must_use]
    pub const fn standard() -> Self {
        Self::for_baud_rate_unchecked(DEFAULT_BAUD_RATE)
    }

    /// The time one octet takes on the wire: ten bit times, rounded up to a
    /// whole microsecond.
    ///
    /// Every time derived from the baud rate is a whole number of
    /// microseconds, rounded up for a lower bound and down for an upper one:
    /// the octet time, T_frame_abort, T_frame_gap, T_postdrive and
    /// T_turnaround, and validation's bounds on them. That keeps the
    /// arithmetic in 32 bits, with no 64-bit division on a 32-bit target.
    #[must_use]
    pub const fn octet_time(&self) -> Duration {
        or_max(ceil_bit_times(self.baud_rate, 10))
    }

    /// Validate all protocol and local bounds.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.baud_rate == 0 {
            return Err(ConfigError::ZeroBaudRate);
        }
        if self.n_poll != 50 {
            return Err(ConfigError::InvalidPollCount { value: self.n_poll });
        }
        if self.n_retry_token != 1 {
            return Err(ConfigError::InvalidTokenRetryCount {
                value: self.n_retry_token,
            });
        }
        if self.n_min_octets != 4 {
            return Err(ConfigError::InvalidMinimumOctets {
                value: self.n_min_octets,
            });
        }
        if self.max_master > MAX_MASTER_ADDRESS {
            return Err(ConfigError::MaxMasterOutOfRange {
                value: self.max_master,
            });
        }
        if self.station > self.max_master {
            return Err(ConfigError::StationExceedsMaxMaster {
                station: self.station,
                max_master: self.max_master,
            });
        }
        if self.max_info_frames == 0 {
            return Err(ConfigError::MaxInfoFramesOutOfRange {
                value: self.max_info_frames,
            });
        }

        let bit_times = |bound: Option<Duration>| bound.ok_or(ConfigError::TimingOverflow);
        let frame_abort_min = bit_times(ceil_bit_times(self.baud_rate, 60))?;
        let frame_gap_max = bit_times(floor_bit_times(self.baud_rate, 20))?;
        let postdrive_max = bit_times(floor_bit_times(self.baud_rate, 15))?;
        let turnaround_min = bit_times(ceil_bit_times(self.baud_rate, 40))?;
        if self.t_frame_abort < frame_abort_min || self.t_frame_abort > MAX_FRAME_ABORT {
            return Err(ConfigError::FrameAbortOutOfRange {
                value: self.t_frame_abort,
                minimum: frame_abort_min,
                maximum: MAX_FRAME_ABORT,
            });
        }
        if self.t_frame_gap > frame_gap_max {
            return Err(ConfigError::FrameGapTooLong {
                value: self.t_frame_gap,
                maximum: frame_gap_max,
            });
        }
        if self.t_postdrive > postdrive_max {
            return Err(ConfigError::PostdriveTooLong {
                value: self.t_postdrive,
                maximum: postdrive_max,
            });
        }
        if self.t_reply_delay > Duration::from_millis(250) {
            return Err(ConfigError::ReplyDelayTooLong {
                value: self.t_reply_delay,
                maximum: Duration::from_millis(250),
            });
        }
        if self.t_no_token != Duration::from_millis(500) {
            return Err(ConfigError::InvalidNoToken {
                value: self.t_no_token,
            });
        }
        if self.reply_transmit_margin >= self.t_reply_delay {
            return Err(ConfigError::ReplyMarginTooLong {
                margin: self.reply_transmit_margin,
                reply_delay: self.t_reply_delay,
            });
        }
        let reply_timeout_max = Duration::from_millis(300);
        if self.t_reply_timeout < MIN_REPLY_TIMEOUT || self.t_reply_timeout > reply_timeout_max {
            return Err(ConfigError::ReplyTimeoutOutOfRange {
                value: self.t_reply_timeout,
                minimum: MIN_REPLY_TIMEOUT,
                maximum: reply_timeout_max,
            });
        }
        if self.t_slot != Duration::from_millis(10) {
            return Err(ConfigError::InvalidSlot { value: self.t_slot });
        }
        if self.t_turnaround < turnaround_min {
            return Err(ConfigError::TurnaroundTooShort {
                value: self.t_turnaround,
                minimum: turnaround_min,
            });
        }
        if self.t_turnaround > self.t_usage_delay {
            return Err(ConfigError::TurnaroundExceedsUsageDelay {
                turnaround: self.t_turnaround,
                usage_delay: self.t_usage_delay,
            });
        }
        if self.t_usage_delay > Duration::from_millis(15) {
            return Err(ConfigError::UsageDelayTooLong {
                value: self.t_usage_delay,
                maximum: Duration::from_millis(15),
            });
        }
        let usage_timeout_max = Duration::from_millis(35);
        if self.t_usage_timeout < MIN_USAGE_TIMEOUT || self.t_usage_timeout > usage_timeout_max {
            return Err(ConfigError::UsageTimeoutOutOfRange {
                value: self.t_usage_timeout,
                minimum: MIN_USAGE_TIMEOUT,
                maximum: usage_timeout_max,
            });
        }
        if self.queue_max_age == Duration::ZERO {
            return Err(ConfigError::ZeroLocalDuration {
                field: LocalDurationField::QueueMaxAge,
            });
        }
        if self.transmit_watchdog == Duration::ZERO {
            return Err(ConfigError::ZeroLocalDuration {
                field: LocalDurationField::TransmitWatchdog,
            });
        }
        if self.transmit_watchdog < self.minimum_transmit_watchdog() {
            return Err(ConfigError::TransmitWatchdogTooShort {
                value: self.transmit_watchdog,
                minimum: self.minimum_transmit_watchdog(),
            });
        }
        if self.broadcast_unicast_ratio == 0 {
            return Err(ConfigError::ZeroLocalLimit {
                field: LocalLimitField::BroadcastUnicastRatio,
            });
        }
        if self.multiple_frame_ratio == 0 {
            return Err(ConfigError::ZeroLocalLimit {
                field: LocalLimitField::MultipleFrameRatio,
            });
        }
        if self.multiple_frame_ratio > 100 {
            return Err(ConfigError::MultipleFrameRatioOutOfRange {
                value: self.multiple_frame_ratio,
            });
        }
        if self.starvation_limit == 0 {
            return Err(ConfigError::ZeroLocalLimit {
                field: LocalLimitField::StarvationLimit,
            });
        }
        Ok(())
    }

    const fn for_baud_rate_unchecked(baud_rate: u32) -> Self {
        let config = Self {
            station: 0,
            max_master: MAX_MASTER_ADDRESS,
            max_info_frames: 1,
            baud_rate,
            n_poll: 50,
            n_retry_token: 1,
            n_min_octets: 4,
            t_frame_abort: or_max(ceil_bit_times(baud_rate, 60)),
            t_frame_gap: or_max(floor_bit_times(baud_rate, 20)),
            t_no_token: Duration::from_millis(500),
            t_postdrive: or_max(floor_bit_times(baud_rate, 15)),
            t_reply_delay: Duration::from_millis(250),
            t_reply_timeout: MIN_REPLY_TIMEOUT,
            t_slot: Duration::from_millis(10),
            t_turnaround: or_max(ceil_bit_times(baud_rate, 40)),
            t_usage_delay: Duration::from_millis(15),
            t_usage_timeout: MIN_USAGE_TIMEOUT,
            reply_transmit_margin: DEFAULT_REPLY_TRANSMIT_MARGIN,
            queue_max_age: Duration::from_secs(5),
            broadcast_unicast_ratio: 1,
            multiple_frame_ratio: 100,
            starvation_limit: 8,
            transmit_watchdog: DEFAULT_TRANSMIT_WATCHDOG,
            extended_frames: false,
        };
        config.with_covering_watchdog()
    }

    /// The longest NPDU this configuration sends or takes: 1497 octets with
    /// [`Self::extended_frames`] on (Clauses 9.3.9, 9.3.10), else 501
    /// (Clauses 9.3.6, 9.3.7).
    #[must_use]
    pub const fn max_npdu_length(&self) -> usize {
        if self.extended_frames {
            MAX_EXTENDED_MPDU_DATA
        } else {
            MAX_STANDARD_MPDU_DATA
        }
    }

    /// The time the longest frame this configuration sends takes on the
    /// wire: [`MAX_EXTENDED_FRAME_LENGTH`] octets with
    /// [`Self::extended_frames`] on, else [`MAX_STANDARD_FRAME_LENGTH`].
    #[must_use]
    pub const fn longest_frame_time(&self) -> Duration {
        let octets = if self.extended_frames {
            MAX_EXTENDED_FRAME_LENGTH
        } else {
            MAX_STANDARD_FRAME_LENGTH
        };
        // At most 1516 octets, so the cast is exact.
        self.octet_time().saturating_mul(octets as u32)
    }

    /// The shortest [`Self::transmit_watchdog`] validation accepts: the
    /// longest frame's time on the wire ([`Self::longest_frame_time`]) plus
    /// T_postdrive, which the watchdog's span from the confirmed start to
    /// completion also covers, plus 1 ms for the adapter to report the
    /// completion, the lateness at which the core counts a deadline missed
    /// ([`LATE_DEADLINE`](crate::LATE_DEADLINE)). A watchdog at this floor
    /// leaves nothing more for a slow adapter; the constructors' defaults
    /// leave a second or more.
    #[must_use]
    pub const fn minimum_transmit_watchdog(&self) -> Duration {
        self.longest_frame_time()
            .saturating_add(self.t_postdrive)
            .saturating_add(WATCHDOG_COMPLETION_MARGIN)
    }

    /// This configuration with [`Self::extended_frames`] on, and a transmit
    /// watchdog that clears the longest extended frame: the one it has if
    /// that clears it already, otherwise that frame's time on the wire plus
    /// one second.
    #[must_use]
    pub const fn with_extended_frames(mut self) -> Self {
        self.extended_frames = true;
        self.with_covering_watchdog()
    }

    /// This configuration, with a transmit watchdog that clears
    /// [`Self::minimum_transmit_watchdog`]: the one it has if it does, else
    /// the longest frame's time plus the default one second.
    const fn with_covering_watchdog(mut self) -> Self {
        if self.transmit_watchdog.as_nanos() < self.minimum_transmit_watchdog().as_nanos() {
            self.transmit_watchdog = self
                .longest_frame_time()
                .saturating_add(DEFAULT_TRANSMIT_WATCHDOG);
        }
        self
    }
}

/// `bits` bit times at `baud_rate`, rounded up to a whole microsecond, for
/// minimum bounds. `None` for a zero baud rate or on overflow.
///
/// The arithmetic is 32-bit: every caller asks for at most 60 bit times,
/// whose 60,000,000 microsecond-bits fit a `u32`, so the result is exact and
/// a 32-bit target links no 64-bit division routine for it. Nanosecond steps
/// would need 60,000,000,000, past a `u32`, and whole microseconds keep each
/// value what it was before the core counted nanoseconds.
const fn ceil_bit_times(baud_rate: u32, bits: u32) -> Option<Duration> {
    match bits.checked_mul(1_000_000) {
        Some(numerator) if baud_rate != 0 => {
            Some(Duration::from_micros(numerator.div_ceil(baud_rate) as u64))
        }
        _ => None,
    }
}

/// `bits` bit times at `baud_rate`, rounded down to a whole microsecond, for
/// maximum bounds. `None` for a zero baud rate or on overflow. 32-bit, as
/// [`ceil_bit_times`] is.
const fn floor_bit_times(baud_rate: u32, bits: u32) -> Option<Duration> {
    match bits.checked_mul(1_000_000) {
        Some(numerator) => match numerator.checked_div(baud_rate) {
            Some(micros) => Some(Duration::from_micros(micros as u64)),
            None => None,
        },
        None => None,
    }
}

/// A bit-time default for [`MstpCoreConfig::for_baud_rate_unchecked`]: a
/// zero baud rate gives [`Duration::MAX`], which `validate` then rejects.
const fn or_max(duration: Option<Duration>) -> Duration {
    match duration {
        Some(duration) => duration,
        None => Duration::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigError, MstpCoreConfig};
    use bacnet_time::Duration;

    #[test]
    fn defaults_are_standard_safe() {
        let config = MstpCoreConfig::default();
        assert_eq!(config.baud_rate, 9_600);
        assert_eq!(config.n_poll, 50);
        assert_eq!(config.n_retry_token, 1);
        assert_eq!(config.n_min_octets, 4);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_rejects_zero_baud_and_bad_addresses() {
        let config = MstpCoreConfig {
            baud_rate: 0,
            ..MstpCoreConfig::default()
        };
        assert_eq!(config.validate(), Err(ConfigError::ZeroBaudRate));

        let config = MstpCoreConfig {
            max_master: 127,
            station: 128,
            ..MstpCoreConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::StationExceedsMaxMaster { .. })
        ));
    }

    #[test]
    fn validation_enforces_bit_time_edges_without_overflow() {
        let config = MstpCoreConfig {
            t_frame_abort: Duration::from_millis(1),
            ..MstpCoreConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::FrameAbortOutOfRange { .. })
        ));

        let config = MstpCoreConfig {
            baud_rate: 1,
            ..MstpCoreConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::FrameAbortOutOfRange { .. })
        ));
    }

    /// The 32-bit bit-time arithmetic gives exactly what 64-bit division
    /// gives, for every bit count it is used for, at every baud rate.
    #[test]
    fn bit_times_match_64_bit_division() {
        for baud in [
            1,
            7,
            9_600,
            19_200,
            38_400,
            57_600,
            76_800,
            115_200,
            1_000_003,
            u32::MAX,
        ] {
            for bits in [10u32, 15, 20, 40, 60] {
                let numerator = u64::from(bits) * 1_000_000;
                let baud64 = u64::from(baud);
                assert_eq!(
                    super::ceil_bit_times(baud, bits),
                    Some(Duration::from_micros(numerator.div_ceil(baud64))),
                    "ceil {bits} at {baud}"
                );
                assert_eq!(
                    super::floor_bit_times(baud, bits),
                    Some(Duration::from_micros(numerator / baud64)),
                    "floor {bits} at {baud}"
                );
            }
        }
        assert_eq!(super::ceil_bit_times(0, 10), None);
        assert_eq!(super::floor_bit_times(0, 10), None);
    }

    #[test]
    fn baud_constructor_recomputes_bit_time_fields() {
        let config = MstpCoreConfig::for_baud_rate(19_200).expect("valid baud");
        assert!(config.validate().is_ok());
        assert!(config.t_turnaround.as_micros() >= 40_000_000 / 19_200);
    }

    #[test]
    fn validation_rejects_infeasible_response_and_ratio_policies() {
        let config = MstpCoreConfig {
            t_turnaround: Duration::from_millis(16),
            ..MstpCoreConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::TurnaroundExceedsUsageDelay { .. })
        ));

        let config = MstpCoreConfig {
            multiple_frame_ratio: 101,
            ..MstpCoreConfig::default()
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::MultipleFrameRatioOutOfRange { value: 101 })
        );
    }

    /// The watchdog must outlast the longest frame plus T_postdrive and the
    /// 1 ms completion margin, with extended frames on or off. At 9600 baud
    /// one second clears a standard frame (511 octets, 0.53 s) but not an
    /// extended one (1516 octets, 1.58 s): `with_extended_frames` raises it
    /// to that frame's time plus one second, and leaves a longer one alone.
    #[test]
    fn the_watchdog_must_outlast_the_longest_frame() {
        let standard = MstpCoreConfig::default();
        assert!(!standard.extended_frames);
        assert_eq!(standard.transmit_watchdog, Duration::from_secs(1));
        assert_eq!(
            standard.longest_frame_time(),
            Duration::from_micros(511 * 1_042)
        );
        // T_postdrive at 9600 baud is 15 bit times, 1562 us.
        assert_eq!(
            standard.minimum_transmit_watchdog(),
            Duration::from_micros(511 * 1_042 + 1_562 + 1_000)
        );

        let flagged = MstpCoreConfig {
            extended_frames: true,
            ..MstpCoreConfig::default()
        };
        let longest = Duration::from_micros(1_516 * 1_042);
        let minimum = longest + Duration::from_micros(1_562 + 1_000);
        assert_eq!(flagged.longest_frame_time(), longest);
        assert_eq!(flagged.minimum_transmit_watchdog(), minimum);
        assert_eq!(
            flagged.validate(),
            Err(ConfigError::TransmitWatchdogTooShort {
                value: Duration::from_secs(1),
                minimum,
            })
        );
        // Exactly at the boundary: one microsecond short is refused, the
        // floor itself accepted.
        let short = MstpCoreConfig {
            transmit_watchdog: minimum - Duration::from_micros(1),
            ..flagged
        };
        assert_eq!(
            short.validate(),
            Err(ConfigError::TransmitWatchdogTooShort {
                value: minimum - Duration::from_micros(1),
                minimum,
            })
        );
        let at_floor = MstpCoreConfig {
            transmit_watchdog: minimum,
            ..flagged
        };
        assert_eq!(at_floor.validate(), Ok(()));

        let extended = MstpCoreConfig::default().with_extended_frames();
        assert!(extended.extended_frames);
        assert_eq!(extended.transmit_watchdog, longest + Duration::from_secs(1));
        assert_eq!(extended.validate(), Ok(()));
        let patient = MstpCoreConfig {
            transmit_watchdog: Duration::from_secs(9),
            ..MstpCoreConfig::default()
        }
        .with_extended_frames();
        assert_eq!(patient.transmit_watchdog, Duration::from_secs(9));
        // Fast enough for one second to clear the extended floor: kept.
        let fast = MstpCoreConfig::for_baud_rate(115_200)
            .unwrap()
            .with_extended_frames();
        assert_eq!(fast.transmit_watchdog, Duration::from_secs(1));
        assert_eq!(fast.validate(), Ok(()));
    }

    /// Below about 5.1 kbaud the longest standard frame alone outlasts one
    /// second (511 octets take 1.06 s at 4800 baud), so the rule applies
    /// without extended frames too. The constructors then default to that
    /// frame's time plus one second, and a hand-built one-second watchdog
    /// is refused. At the standard rates nothing changes.
    #[test]
    fn low_baud_rates_get_a_watchdog_that_outlasts_a_standard_frame() {
        let slow = MstpCoreConfig::for_baud_rate(4_800).unwrap();
        // One octet takes 2084 us, T_postdrive 3125 us.
        let longest = Duration::from_micros(511 * 2_084);
        assert_eq!(slow.longest_frame_time(), longest);
        assert_eq!(slow.transmit_watchdog, longest + Duration::from_secs(1));
        assert!(slow.transmit_watchdog >= Duration::from_millis(1_070));
        assert_eq!(slow.validate(), Ok(()));

        let hand_built = MstpCoreConfig {
            transmit_watchdog: Duration::from_secs(1),
            ..slow
        };
        assert_eq!(
            hand_built.validate(),
            Err(ConfigError::TransmitWatchdogTooShort {
                value: Duration::from_secs(1),
                minimum: longest + Duration::from_micros(3_125 + 1_000),
            })
        );
        // The lowest rate validation takes still gets a valid default.
        assert!(MstpCoreConfig::for_baud_rate(2_667).is_ok());
        for baud in [9_600, 19_200, 38_400, 57_600, 76_800, 115_200] {
            let config = MstpCoreConfig::for_baud_rate(baud).unwrap();
            assert_eq!(config.transmit_watchdog, Duration::from_secs(1), "{baud}");
        }
    }

    #[test]
    fn reply_margin_must_leave_part_of_the_reply_delay() {
        let config = MstpCoreConfig::default();
        assert_eq!(config.reply_transmit_margin, Duration::from_millis(25));

        let config = MstpCoreConfig {
            reply_transmit_margin: Duration::from_millis(250),
            ..MstpCoreConfig::default()
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::ReplyMarginTooLong {
                margin: Duration::from_millis(250),
                reply_delay: Duration::from_millis(250),
            })
        );

        let config = MstpCoreConfig {
            t_reply_delay: Duration::from_millis(100),
            reply_transmit_margin: Duration::from_micros(99_999),
            ..MstpCoreConfig::default()
        };
        assert_eq!(config.validate(), Ok(()));
    }
}
