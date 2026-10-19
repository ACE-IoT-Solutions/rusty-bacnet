---
section: Changed
---
- **Breaking (Rust API):** A log record's failure holds `ErrorClass` and
  `ErrorCode`, and a decoded failure wider than 16 bits is refused, as an
  Error PDU's is (#1814).
