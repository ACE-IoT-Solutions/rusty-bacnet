---
section: Added
---
- Test-only: CI links `bacnet-encoding` and `bacnet-services` without `std`
  into a third bare-metal image, with a global allocator and a single-core
  critical section, for Cortex-M0, Cortex-M3 and RV32IMC, and checks its
  flash and RAM against a size budget; a native test runs its scenario (#1635).
