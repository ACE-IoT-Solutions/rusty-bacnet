---
section: Migration notes
---
- **B/IPv6 MAC (Rust API, wire, #1637):** Pass a peer's 3-octet VMAC wherever
  an 18-octet B/IPv6 MAC went: sends, client requests, recipient lists,
  bindings and stored COV subscriptions. Find a VMAC from an address with
  `Bip6Transport::resolve_virtual_address`, and read this node's address with
  `local_address()`. Upgrade a router together with the B/IPv6 nodes it serves.
