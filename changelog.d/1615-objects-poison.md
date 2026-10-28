---
section: Changed
---
- The Audit Reporter status and association locks and the device's EventSequence carry on after a panic poisoned them instead of panicking on every later call; a panic inside an EventSequence transaction closure now leaves the number unconsumed (#1615).
