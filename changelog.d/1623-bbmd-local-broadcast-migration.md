---
section: Migration notes
---
- **BbmdState (Rust API, #1623):** `forwarded_npdu_needs_local_broadcast`
  now answers for a unicast forward by this BBMD's own BDT row: true when
  that row is two-hop, whatever the peer's row says. It is false for the
  BBMD's own address, and for a peer on its subnet at its port once the
  transport has the subnet mask. Code that read the peer's mask from it
  should read the peer's BDT row instead.
