---
section: Added
---
- Test-only: a manual check runs the firmware link test's images under QEMU
  on Cortex-M0, M3 and M4F and RV32 boards, checks each scenario's summary
  against its host test's and records peak stack use; a script and a
  manually started workflow run it, never CI (#1651).
