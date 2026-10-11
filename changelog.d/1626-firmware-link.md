---
section: Added
---
- CI: the MS/TP codec and master core are linked into a bare-metal image with
  no allocator for Cortex-M0, Cortex-M3 and RV32IMC, and each image's flash
  and RAM use is checked against a size budget; a native test runs the image's
  scenario (#1626).
