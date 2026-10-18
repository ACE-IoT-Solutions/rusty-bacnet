---
section: Migration notes
---
- **bacnet-time versioning (#1768):** a direct dependency on `bacnet-time`
  names that crate's own version, not the release's: a release may leave it
  at an earlier number. The other crates already name the one they need.
