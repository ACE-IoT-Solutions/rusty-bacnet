---
section: Changed
---
- **Breaking (Rust API):** `BipPortMode::Foreign` lends a live
  `ForeignRegistration` view instead of a fixed BBMD and lifetime, and
  `ConfigError` gains `NotForeign` (#1825).
