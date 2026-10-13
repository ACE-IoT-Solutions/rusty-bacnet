---
section: Changed
---
- **Breaking (Rust API):** `bacnet-time`'s `DurationTooLong` comes only
  from the `TryFrom` conversion, so it can carry more detail later, and
  `Duration::is_zero` is new (#1684).
