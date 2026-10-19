---
section: Changed
---
- **Breaking (Rust API):** Protocol errors, rejects and aborts carry the typed
  error class, code and reason in `Error` and the client's `TsmResponse`, and
  the analog, accumulator, pulse converter, loop and staging objects take
  `EngineeringUnits`. The wire and the Python API are unchanged (#915).
