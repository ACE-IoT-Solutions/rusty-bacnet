---
section: Migration notes
---
- **B/IP core (Rust API, #1684):** `RegistrationView` is
  `RegistrationSnapshot`, `GRACE_PERIOD_SECS` is the `Duration`
  `GRACE_PERIOD`, and `StorageFull` and `BdtError::Capacity` carry `needed`
  and `available`. Build `Datagram` and `Context` with `new`; start `Link`,
  `Foreign` and `RegistrationLimits` from `new` or `DEFAULT`, and counters
  from `Default`, then set fields. A `match` on an error, refusal or
  `Discard` needs a wildcard arm
  ([every type](docs/rust-api.md#bacnet-bip-core-firmware)).
