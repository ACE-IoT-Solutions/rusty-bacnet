---
section: Changed
---
- **Python API:** `create_object` returns the created `ObjectIdentifier`, and
  the CLI's `create-object` prints it (with `object_type` and `instance` in
  JSON) instead of the acknowledgment's hex; bacnet-services gains
  `CreateObjectAck` (#2080).
