---
section: Migration notes
---
- **bacnet-time (Rust API, #1684):** `DurationTooLong` can no longer be
  built by hand: match `Err(DurationTooLong { .. })` instead of comparing
  against a value.
