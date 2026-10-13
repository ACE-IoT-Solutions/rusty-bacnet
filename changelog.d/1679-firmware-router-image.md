---
section: Added
---
- Test-only: CI links a router from `bacnet-router-core`, between a BBMD's
  B/IP port and an MS/TP port with extended frames, into a bare-metal image
  with no allocator and checks its flash and RAM against size budgets; a
  native test runs its scenario, and the manual emulation check its stack
  (#1679).
