---
section: Migration notes
---
- **NAT traversal (Rust API, #940):** a `BbmdOptions` literal naming every
  field adds `nat: None` or ends with `..BbmdOptions::default()`; a `match`
  on `DataLinkSupport` adds a wildcard arm; build `Tables` with
  `Tables::new`; `BbmdState::local_address()` returns a `SocketAddrV4`
  (was `([u8; 4], u16)`), and `own_address()` is the BDT's own address.
