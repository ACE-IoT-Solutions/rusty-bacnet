---
section: Fixed
---
- **Python API:** The first `connection_events()` call on an SC
  `BACnetClient` now starts with the first `connected` event; later calls
  still start from the call (#1908).
