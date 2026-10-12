---
section: Fixed
---
- A full B/IPv6 VMAC table evicts the peer heard from least recently instead
  of ignoring new peers and never learns this node's own VMAC; frames from
  another node using that VMAC, and NPDUs from a source no reply could reach,
  are dropped and counted (#1637).
