---
section: Added
---
- **Python API:** `BACnetServer` and the endpoints take `on_reinitialize`
  and `after_reinitialize` to carry out ReinitializeDevice, which also makes a
  BBMD port's rows writable; endpoints take `reinit_password`, and
  `report_device_reset_local()` records an application reset (#1566).
