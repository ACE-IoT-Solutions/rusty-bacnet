---
section: Migration notes
---
- **Router (Rust API, #2002):** Handle the result of `router.stop().await`, now `Result<(), RouterStopError>`. Only success guarantees completed cleanup. Repeated calls replay the same result; inspect `failures()` for original transport errors. Cancellation or Drop alone does not guarantee endpoint reuse.
