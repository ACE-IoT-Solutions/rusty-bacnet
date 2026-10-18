---
section: Added
---
- `bacnet-types`, `bacnet-encoding` and `bacnet-services` build without an
  allocator: Who-Is, I-Am and ReadProperty encode into caller storage and
  decode in place, with string sinks, borrowed value views and an APDU header
  writer (#1614).
