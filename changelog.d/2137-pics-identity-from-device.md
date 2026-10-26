---
section: Fixed
---
- **Breaking (Rust API):** A generated PICS takes its vendor block (the vendor
  ID, the vendor and model names and both versions) from the Device it
  describes, so it can't print blank or stale details; `PicsConfig` loses its
  four product fields (#2137).
