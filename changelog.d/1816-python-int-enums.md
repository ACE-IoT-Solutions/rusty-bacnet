---
section: Changed
---
- **Python API:** `EngineeringUnits`, `ErrorClass`, `ErrorCode`, `RejectReason`
  and `AbortReason` are `IntEnum` classes: `units=` takes a member or an int,
  and raised errors' `error_class`, `error_code` and `reason` are members that
  equal the wire integers (#1816).
