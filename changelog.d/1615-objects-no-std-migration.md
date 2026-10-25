---
section: Migration notes
---
- **bacnet-objects features (Rust API, #1615):** with `default-features = false`, pick `features = ["std"]` on hosted targets. `["alloc"]` is the no_std build: no `File*Persistence` backends, saves inline under the caller's guard, no wall-clock expiry of staged writes and no tracing. Defaults are unchanged.
