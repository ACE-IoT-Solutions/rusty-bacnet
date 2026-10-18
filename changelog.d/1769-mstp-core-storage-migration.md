---
section: Migration notes
---
- **MS/TP core (Rust API, #1769):** build the core with
  `MasterCore::new(config, &mut storage, now)` over a `StandardStorage` or
  `ExtendedStorage` (`from_parts` keeps the old arguments); rename
  `HeaplessQueues` to `FixedQueues`; drop `?`/`unwrap` on the `on_*` line
  inputs and watch `adapter_errors`; match `ConfigError`, `TransmitError`
  or `RequestError` where you matched `CoreError`.
