---
section: Changed
---
- **Breaking (Rust API):** `bacnet-mstp-core` borrows its frame and
  delivery buffers (`ActionBuffers`) as it does its receive storage, which
  cuts the firmware image's RAM by 8.9 KB; its control actions can't fill
  up, and it divides only in 32 bits, saving up to 0.9 KB of flash (#1618).
