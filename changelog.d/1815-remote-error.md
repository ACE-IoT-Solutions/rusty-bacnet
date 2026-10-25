---
section: Changed
---
- **Breaking (Rust API):** A peer's Error, Reject or Abort PDU is one
  `RemoteError` from the wire to the API: `Error::Remote`, the client's
  `TsmResponse::Remote` and the server's `CovAckResult::Error` all hold it
  (#1815).
