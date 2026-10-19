---
section: Changed
---
- The `stress-test` mixed and router scenarios connect every client before
  the clock starts, keep one histogram per client and divide by the measured
  step time, as the clients scenario does (#1817).
