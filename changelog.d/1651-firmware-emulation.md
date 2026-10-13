---
section: Added
---
- Test-only: a manual check, never run by CI, runs the firmware images
  under QEMU on Cortex-M0, M3 and M4F and RV32 boards, compares each
  scenario's summary with its host test's, and fails when an image's stack
  peak outgrows its budget (#1651, #1660).
