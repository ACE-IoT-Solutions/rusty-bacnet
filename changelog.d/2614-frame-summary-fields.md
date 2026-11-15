---
section: Migration notes
---
- **Breaking (Rust API, #2614):** Exhaustive `FrameSummary` and `Link` literals must initialize the added observation metadata fields. Prefer `summarize_frame` and `link` to construct decoder results; partial `FrameSummary` literals can use `..Default::default()`. The ACE decoder remains in `bacnet_ops::decode`.
