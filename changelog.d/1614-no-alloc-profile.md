---
section: Added
---
- `bacnet-types`, `bacnet-encoding` and `bacnet-services` build without an
  allocator: Who-Is, I-Am and ReadProperty encode into caller storage and
  decode in place, and tags, scalars, timestamps and references encode whole
  or not at all into slices, `heapless`, `Vec` or `BytesMut` sinks (#1614).
