---
section: Migration notes
---
- **BBMD options and NAT (Rust API, #940, #1668):** a `BbmdOptions` literal
  naming every field adds `nat: None` and `alone_on_subnet: false`, or ends
  with `..BbmdOptions::default()`; a `match` on `DataLinkSupport` adds a
  wildcard arm; build `Tables` with `Tables::new`;
  `BbmdState::local_address()` returns a `SocketAddrV4` (was
  `([u8; 4], u16)`), and the new `own_address()` gives the BDT's own address.
