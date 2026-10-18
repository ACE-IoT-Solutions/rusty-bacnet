---
section: Migration notes
---
- **APDU header (Rust API, #1771):** `bacnet_encoding::apdu::header::ApduHeader`
  is `#[non_exhaustive]` and gains `Error`, `Reject` and `Abort`, which
  `ApduHeader::decode` now returns for those PDU types instead of an
  `Unsupported` decoding error; give a `match` on it a wildcard arm.
