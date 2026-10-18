---
section: Migration notes
---
- **BBMD port (Rust API, #1766):** `BbmdPort` runs the BBMD's `Link`, not a
  `Node`, so a BVLC-Result or table ACK reaching it is `Ingress::Handled`
  and no longer counts in `NodeCounters::unmatched_responses`; a BBMD that
  runs exchanges of its own drives a `Node` beside the port.
