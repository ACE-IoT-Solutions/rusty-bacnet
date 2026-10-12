---
section: Changed
---
- **Breaking (Rust API):** A `BipTransport` set up both as a BBMD and as a
  foreign device now fails `start()` instead of mixing the two modes (#1622).
