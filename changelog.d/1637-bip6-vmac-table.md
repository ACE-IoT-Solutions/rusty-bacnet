---
section: Fixed
---
- A full B/IPv6 VMAC table evicts the peer heard from least recently instead
  of ignoring new peers, never learns this node's own VMAC, and drops frames
  from another node using it (#1637).
