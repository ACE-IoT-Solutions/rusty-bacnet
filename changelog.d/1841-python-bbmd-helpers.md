---
section: Added
---
- **Python API:** `BACnetClient` reads and writes a BBMD's tables, deletes an
  FDT entry and registers once as a foreign device; a NAK raises
  `BacnetBvlcNakError` with its result code, apart from `BacnetTimeoutError`
  for no reply (#1841).
