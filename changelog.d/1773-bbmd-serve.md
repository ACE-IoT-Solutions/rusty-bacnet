---
section: Changed
---
- **Wire:** The hosted BBMD answers Register-Foreign-Device, Read-BDT,
  Read-FDT and Delete-FDT-Entry through `bacnet-bip-core`'s `Tables::serve`.
  A Delete-FDT-Entry naming a registration past its time-to-live and grace
  period now gets the NAK, since that entry no longer exists (#1773).
