---
section: Migration notes
---
- **B/IP node (Rust API, #1678):** `bacnet_bip_core::node::Datagram::to` is
  an `Option<Addressed>`: wrap what `Addressed::classify` returns in
  `Some`, or pass `None` when the IP stack can't report a datagram's
  destination. A `match` on `Discard` needs an arm for `UnknownDestination`.
