---
section: Changed
---
- **Breaking (Rust API):** `ActionBuffers`, `MasterCore`, `MasterCore::new`
  and `AllocQueues` take buffer sizes as const parameters with standard
  defaults, `MstpCoreConfig` gains `extended_frames`, and `QueueStorage`
  gains `max_npdu`, whose default of 501 suits standard slots
  (#1611).
