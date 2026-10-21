---
section: Migration notes
---
- **MS/TP execution mode (Rust API, #501):** `MstpExecutionMode::DedicatedThread`
  now carries an `MstpThreadConfig`, and the enum is `Clone` but no longer
  `Copy`. Write `MstpExecutionMode::dedicated_thread()` for the old behaviour,
  and clone a mode you use more than once.
