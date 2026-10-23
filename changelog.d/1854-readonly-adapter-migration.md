---
section: Migration notes
---
- **MS/TP monitor (Rust API, #1854):** Await `MstpMonitor::start`, pass a `MonitorRead` input, and retain a mutable monitor for `stop(&mut self)`. Wrap synchronously dropped in-memory readers in `DropOnClose`; deferred native readers must implement awaited close. Resume cancelled stops before reopening a port; Drop alone does not confirm release.
