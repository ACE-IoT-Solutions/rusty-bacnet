---
section: Migration notes
---
- **MS/TP core (Rust API, #1611):** add `extended_frames` to a
  `MstpCoreConfig` literal that lists every field, add `FRAME` to a
  `MasterCore::new::<DELIVERIES>` turbofish (`::<DELIVERIES, FRAME>`, 511
  for standard buffers), write `AllocQueues::<501>::new` where the slot size
  can't be inferred, and match the new `ConfigError` variants. For extended
  frames use `with_extended_frames()`, `ExtendedActionBuffers` and queues
  whose `max_npdu` is 1497.
