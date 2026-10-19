---
section: Migration notes
---
- **BbmdOptions (Rust API, #1668):** `BbmdOptions` has a new
  `alone_on_subnet: bool` field. A struct literal that names every field adds
  `alone_on_subnet: false`, or ends with `..BbmdOptions::default()`.
