---
section: Migration notes
---
- **B/IP core (Rust API, #1684):** renamed: `RegistrationView` to
  `RegistrationSnapshot`, `InvalidEntry` to `EntryError`,
  `GRACE_PERIOD_SECS` to `GRACE_PERIOD` (a `Duration`); `StorageFull` and
  `BdtError::Capacity` carry `needed`, `available`. Build inputs with `new`,
  `RegistrationLimits` from `DEFAULT`, counters from `Default`. Outputs
  can't be built; `match`es on errors, refusals, drops, outcomes, events
  and settings such as `Retry` or `Path` need `_`
  ([types](docs/rust-api.md#bacnet-bip-core-firmware)).
