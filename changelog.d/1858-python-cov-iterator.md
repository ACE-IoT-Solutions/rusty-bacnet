---
section: Changed
---
- **Python API:** `CovNotificationIterator` counts notifications lost to lag
  in a new `missed` instead of printing to stderr, and ends with "COV
  notifications ended: the client stopped" instead of "channel closed"
  (#1858).
