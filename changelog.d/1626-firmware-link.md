---
section: Added
---
- Test-only: CI links two bare-metal images with no allocator, one with the
  MS/TP codec and master core and one with the B/IP BVLL codec, for Cortex-M0,
  Cortex-M3 and RV32IMC, and checks their flash and RAM against a size budget;
  native tests run each image's scenario (#1626).
