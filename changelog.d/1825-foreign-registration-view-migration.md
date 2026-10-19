---
section: Migration notes
---
- **Network Port foreign mode (Rust API, #1825):** build
  `BipPortMode::Foreign { bbmd, subscription_lifetime }` as
  `BipPortMode::fixed_foreign(bbmd, subscription_lifetime)`, or lend your own
  `ForeignRegistration`; read the values with `registration.bbmd()` and
  `registration.subscription_lifetime()`.
