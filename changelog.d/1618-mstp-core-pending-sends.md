---
section: Fixed
---
- **Breaking (Rust API):** `bacnet-mstp-core` no longer drops a frame when
  line activity breaks its turnaround: the frame waits for a quiet line, so
  a damaged reply no longer costs the token. A token generated after a late
  slot deadline is counted in `missed_generation_slots` (#1618).
