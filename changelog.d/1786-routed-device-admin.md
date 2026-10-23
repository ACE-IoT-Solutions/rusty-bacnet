---
section: Added
---

- CLI `control`, `reinit` and `time-sync` reach routed devices with existing guards. Rust from-device methods preserve DCC/reinit fields and local/UTC time service choices; time synchronization sends once without waiting for a response. Part of #1786.
