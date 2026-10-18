---
section: Added
---
- Tag headers and markers now accept slice, heapless, `Vec` and `BytesMut` sinks
  with atomic appends and typed capacity errors. Existing buffer APIs use the
  same implementation; other encoders are unchanged (part of #1614).
