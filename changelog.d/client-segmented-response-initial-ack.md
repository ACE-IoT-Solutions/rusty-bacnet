---
section: Fixed
---
- **Wire:** The standalone client acknowledges the first segment of a response
  immediately, so reads complete with peers that wait for this acknowledgment
  before sending the rest of a multi-segment window.
