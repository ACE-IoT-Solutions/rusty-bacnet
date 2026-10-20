---
section: Migration notes
---
- **B/IP multicast (Rust API, #941):** build a `BipPort` with
  `BipPort::new(endpoint, mode)` in place of a struct literal, and add a
  group view with `.with_multicast(...)`.
