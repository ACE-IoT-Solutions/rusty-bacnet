---
section: Migration notes
---
- **Router origination (Rust API, #1809):** `OriginateError::ReservedNetwork`
  now means DNET 0 only. A request for DNET 0xFFFF with an empty DADR is sent
  on every port instead of refused, and one with a DADR is refused as the new
  `OriginateError::GlobalWithDadr`. Replace a per-network remote broadcast
  workaround with one global request.
