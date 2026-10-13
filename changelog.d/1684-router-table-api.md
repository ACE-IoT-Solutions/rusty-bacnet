---
section: Changed
---
- **Breaking (Rust API):** `bacnet_network::router_table` follows the
  router core: a `Route` is read through getters, `Table::lookup_mut` is
  gone, `Refusal` is `LearnRefusal`, `apply_reject` takes a
  `RejectReason`, and `Route`, `Pending`, `Learned` and `LearnRefusal` are
  `#[non_exhaustive]` (#1684).
