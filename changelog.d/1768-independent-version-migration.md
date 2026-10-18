---
section: Migration notes
---
- **bacnet-time versioning (#1768):** a direct dependency on `bacnet-time`
  names that crate's own version, 0.13.1 in this release, not the release's:
  later releases may leave it where it is. The other crates already name the
  one they need.
