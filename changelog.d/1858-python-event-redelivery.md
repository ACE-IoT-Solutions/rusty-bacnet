---
section: Fixed
---
- **Python API:** SC event, COV, I-Have and network-report iterators no longer
  lose an item when a read is cancelled mid-delivery, as a timed-out
  `wait_for` does: it comes back on the next read. `CovNotificationIterator`
  gains `missed` (#1858).
