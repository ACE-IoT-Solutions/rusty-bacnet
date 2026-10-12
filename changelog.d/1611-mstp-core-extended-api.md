---
section: Changed
---
- **Breaking (Rust API):** `ActionBuffers`, `MasterCore` and `AllocQueues`
  take buffer sizes as const parameters with standard defaults,
  `QueueStorage` gains `max_npdu_length`, and `MstpCoreConfig` gains
  `extended_frames` (#1611).
