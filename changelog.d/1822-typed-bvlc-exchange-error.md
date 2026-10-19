---
section: Changed
---
- **Breaking (Rust API):** The B/IP BBMD helpers on `BipTransport` and
  `BACnetClient` fail with a typed `BvlcExchangeError` that names a NAK, no
  reply and a local failure apart, and a refused registration, FDT delete
  or BDT write is now an error rather than an `Ok` code (#1822).
