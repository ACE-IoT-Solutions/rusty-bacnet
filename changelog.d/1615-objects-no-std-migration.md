---
section: Migration notes
---
- **bacnet-objects features (Rust API, #1615):** a dependency with `default-features = false` now needs `features = ["std"]` for the hosted crate, or `["alloc"]` for the no_std build, which has no `File*Persistence` backends. Default features are unchanged.
