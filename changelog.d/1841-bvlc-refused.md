---
section: Changed
---
- **Breaking (Rust API):** a BBMD helper the B/IP transport refuses before sending (the same request in flight, or no free exchange slot) fails with `BvlcExchangeError::Refused` carrying the reason, in place of an encoding error (#1841).
