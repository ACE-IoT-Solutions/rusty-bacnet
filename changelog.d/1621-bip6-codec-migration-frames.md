---
section: Migration notes
---
- **B/IPv6 frames (Rust API, #1621):** `Bvlc6Frame`, `decode_bvlc6`,
  `encode_bvlc6` and the per-message `encode_*` helpers become `Bvll6` values
  with `decode_bvll6`/`encode_bvll6`; a Forwarded-NPDU's address and NPDU
  come from `origin()`/`npdu()`, not `decode_forwarded_npdu_payload`.
  `BVLC6_TYPE` is `BVLC_TYPE_BIP6`, and the two header-length constants are
  gone.
