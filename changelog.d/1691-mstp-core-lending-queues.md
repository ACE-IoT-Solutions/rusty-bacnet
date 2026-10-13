---
section: Changed
---
- **Breaking (Rust API):** an MS/TP queue dequeue lends the NPDU from its
  slot instead of copying it into a scratch, and the core encodes a queued
  NPDU's frame from there: about 2 KB less stack with extended frames, and
  no `memmove` in the MS/TP firmware images (#1691).
