---
section: Changed
---
- **Breaking (Rust API):** every crate speaks `bacnet-npdu`'s types, with
  the duplicates gone, and `bacnet-encoding`'s owned `Npdu` decodes and
  encodes through its view; `NetworkPriority` is a closed enum and the
  router's control authorizer sees a typed message type (#1770, #915).
