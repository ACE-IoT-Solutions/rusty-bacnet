---
section: Migration notes
---
- **MS/TP core (Rust API, #1611):** implement
  `QueueStorage::max_npdu_length`, add `extended_frames` to a
  `MstpCoreConfig` literal that lists every field, write
  `AllocQueues::<501>::new` where the slot size can't be inferred, and
  match the new `ConfigError` variants. To turn extended frames on, use
  `with_extended_frames()`, `ExtendedActionBuffers` and 1497-octet queue
  slots.
