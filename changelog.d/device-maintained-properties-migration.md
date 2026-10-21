---
section: Migration notes
---
- **DeviceConfig (Rust API):** `DeviceConfig` has two new fields, `backup_and_restore` and
  `backup_failure_timeout`. Code that builds a `DeviceConfig` with a struct
  literal adds `..DeviceConfig::default()`.
