---
section: Changed
---
- **Breaking (Rust API):** `bacnet-mstp-codec` checks a frame header's
  source and Length before its type, so a COBS Length out of range is
  refused with the new `InvalidCobsLength`, and `decode_frame` refuses an
  extended frame, which `decode_frame_in_place` decodes (#1611).
