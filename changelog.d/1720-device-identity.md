---
section: Changed
---
- **Breaking (Rust API):** Servers and endpoints share one Device identity
  (#1720). Startup resolves receive capacity before generating a Device, so a
  default MS/TP server declares 480. Runtime announcements and served identity
  fields remain coherent after database replacement.
