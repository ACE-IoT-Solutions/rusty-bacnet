---
section: Added
---
- An accepted ReinitializeDevice request writes a DEVICE_RESET,
  DEVICE_BACKUP or DEVICE_RESTORE Audit record on the server and on endpoint
  sessions, and `BACnetServer::report_device_reset_local` records a reset the
  application makes for its own reasons (#1564).
