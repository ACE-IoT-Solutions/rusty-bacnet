---
section: Migration notes
---
- **NetworkPortEntry (Rust API, #1940):** The entry has a new `ipv6` field (zeros except on a B/IPv6 entry). Build entries with `NetworkPortEntry::bip`, `bip6` or `sc`, or add `ipv6: [0; 16]` to a struct literal.
