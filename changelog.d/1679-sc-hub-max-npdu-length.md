---
section: Changed
---
- **Breaking (Rust API):** `ScConnection::hub_max_apdu_length` is now
  `hub_max_npdu_length`, naming what it holds: the hub's Max-NPDU-Length from
  its Connect-Accept (#1679).
