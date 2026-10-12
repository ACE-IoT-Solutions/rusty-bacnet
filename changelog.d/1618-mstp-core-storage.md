---
section: Changed
---
- **Breaking (Rust API):** `bacnet-mstp-core` borrows its frame and
  delivery buffers (`ActionBuffers`) as it does its receive storage, keeps
  its control actions where they can't fill up, and divides only in 32
  bits, so the firmware image needs 8.9 KB less RAM (#1618).
