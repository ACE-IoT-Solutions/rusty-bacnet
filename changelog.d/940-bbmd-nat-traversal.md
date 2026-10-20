---
section: Added
---
- A BBMD can run behind a NAT router that port-forwards its B/IP port, at a
  configured global address (`BbmdOptions::nat`, `NatTraversal`; in
  `bacnet-bip-core`, `Tables::set_nat` and `bbmd::Nat`), with two-hop BDT
  rules and the global origin in the Forwarded-NPDUs it builds (#940).
