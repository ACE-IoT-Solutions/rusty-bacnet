---
section: Changed
---
- **Breaking (Rust API):** The B/IP port driver moves from
  `bacnet_router_core::bip` into `bacnet_bip_core::port` (`NodePort`,
  `BbmdPort`), and `Tables::serve` answers BBMD management requests, so a
  BBMD port answers them itself, Delete-FDT-Entry included (#1766).
