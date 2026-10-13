---
section: Changed
---
- **Breaking (Rust API):** `bacnet_network::router_table` follows the
  core (#1684): `Route` getters, no `lookup_mut` or `new_hardened`,
  `Refusal` as `LearnRefusal`, `apply_reject` taking `RejectReason`
  (re-exported), `#[non_exhaustive]` `Route`, `Pending`, `Learned`,
  `MacTooLong` and `ControlClass`.
