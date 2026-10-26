---
section: Fixed
---
- Test-only: the `bacnet-mstp-sim` adapter can tick its timers at SilenceTimer's coarse resolution from a seeded phase, so adjacent masters at opposite clock edges at 9,600 and 19,200 baud now form a ring after a bounded few collisions instead of locking step (#1804).
