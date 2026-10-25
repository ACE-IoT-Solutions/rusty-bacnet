---
section: Added
---
- `remote_error()` on `ErrorPdu`, `RejectPdu` and `AbortPdu` reads a refusal
  PDU as a `RemoteError`, and the client and server use it instead of mapping
  each PDU by hand; `Apdu::remote_error()` does the same for any APDU (#2026).
