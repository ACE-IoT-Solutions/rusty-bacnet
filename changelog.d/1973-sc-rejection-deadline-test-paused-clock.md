---
section: Changed
---
- Test-only: the BACnet/SC rejection deadline test that checks the budget
  before a re-poll and after a slow completion runs on tokio's paused clock,
  so a stalled runner can no longer expire its 10 ms budget early (#1973).
