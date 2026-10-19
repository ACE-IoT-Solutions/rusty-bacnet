---
section: Changed
---
- The allocation-free profile's doctests now run in CI (#1779): the
  bacnet-services README and the tag-marker examples write to a fixed buffer
  instead of a `BytesMut`, so they build without `alloc`.
