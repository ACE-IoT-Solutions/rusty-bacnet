---
section: Added
---
- `Apdu::remote_error()` and `remote_error()` on `ErrorPdu`, `RejectPdu` and
  `AbortPdu` read a refusal PDU as a `RemoteError`; the client and server use
  them instead of mapping each PDU by hand (#2026).
