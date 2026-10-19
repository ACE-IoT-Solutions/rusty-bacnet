---
section: Changed
---
- **Python API:** A Python server's BBMD port refuses writes to
  BBMD_Broadcast_Distribution_Table and BBMD_Accept_FD_Registrations with
  WRITE_ACCESS_DENIED, since it takes no ReinitializeDevice handler to
  activate them (#1578).
