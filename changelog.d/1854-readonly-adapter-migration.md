---
section: Migration notes
---
- **MS/TP monitor (Rust API, #1854):** Await `MstpMonitor::start`, pass `MonitorRead`, and retain a mutable monitor for `stop`. Use `DropOnClose` only for synchronous in-memory owners. Open/start errors now use `MonitorAdmissionError`; check `release_confirmed()` before retrying. Resume cancelled stops before reopening; Drop or failed stop does not confirm release.
