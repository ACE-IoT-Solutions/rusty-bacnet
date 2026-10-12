---
section: Changed
---
- **Breaking (Rust API):** The B/IP transport runs on the `bacnet-bip-core`
  node engine. A foreign device now refuses manual Register, Delete-FDT-Entry
  and Write-BDT requests to its own BBMD, and `start()` refuses a BBMD
  address no device could register with (#1622).
