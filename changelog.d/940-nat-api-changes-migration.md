---
section: Migration notes
---
- **NAT traversal (Rust API, #940):** a `BbmdOptions` literal naming every
  field adds `nat: None` or ends with `..BbmdOptions::default()`; a `match`
  on `DataLinkSupport` adds `BipV4NatTraversal`; build `Tables` with
  `Tables::new`, not a struct literal. Use `BbmdState::own_address` for the
  address the BDT knows the BBMD by.
