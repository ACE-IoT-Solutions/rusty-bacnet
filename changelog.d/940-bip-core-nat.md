---
section: Added
---
- `bacnet-bip-core` runs a BBMD under a NAT router's global address
  (`Tables::set_nat`, `bbmd::Nat`), with the two-hop and own-row BDT rules
  and the global originating address in the Forwarded-NPDUs it builds (#940).
