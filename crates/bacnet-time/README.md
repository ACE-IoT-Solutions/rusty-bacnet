# bacnet-time

The time the portable cores of
[rusty-bacnet](https://github.com/jscott3201/rusty-bacnet) share: a monotonic
`Instant` and a `Duration`, both counted in nanoseconds on a `u64`.
`bacnet-mstp-core` and `bacnet-bip-core` take it, so firmware that runs both
feeds them one clock.

- `#![no_std]`, no allocator, no dependencies, no features.
- The epoch is yours: the cores read no clock, and each call that depends on
  time takes `now`. A `u64` of nanoseconds lasts about 584 years.
- `+` and `-` saturate, and every operation has a `checked_*` form that
  returns `None` instead. Nothing panics.
- Unit constructors (`from_micros`, `from_millis`, `from_secs`) are `const`
  and saturate; unit accessors (`as_micros`, `as_millis`, `as_secs`) round
  toward zero.
- `From` conversions to and from `core::time::Duration`, so a host maps its
  monotonic clock onto the timeline exactly.
- No 64-bit division: on a 32-bit target, where dividing a `u64` links a
  routine of about 1 KB, the accessors, `checked_rem` and the conversion to
  `core::time::Duration` use binary long division.

## Example

```rust
use bacnet_time::{Duration, Instant};

// Firmware with a millisecond tick.
let now = Instant::from_millis(1_250);
let deadline = now + Duration::from_millis(500);
assert_eq!(deadline.saturating_duration_since(now).as_millis(), 500);

// Saturation, and the checked forms.
assert_eq!(Instant::ZERO - Duration::from_secs(1), Instant::ZERO);
assert_eq!(Instant::ZERO.checked_sub(Duration::from_secs(1)), None);
assert_eq!(now - deadline, Duration::ZERO);

// A host's monotonic clock, through core::time::Duration.
let elapsed = core::time::Duration::from_micros(2_500);
assert_eq!(Duration::from(elapsed), Duration::from_micros(2_500));
assert_eq!(core::time::Duration::from(Duration::from_nanos(1_500)), core::time::Duration::from_nanos(1_500));
```

## License

MIT
