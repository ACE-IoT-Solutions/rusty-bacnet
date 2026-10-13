---
section: Changed
---
- The server's discovery and time-synchronization rate limits, discovery
  coalescing windows, binding freshness, COV subscription lifetimes, Life
  Safety operation replay and the confirmed-event repeat window use
  tokio's clock, so paused tests step them; production is unchanged
  (#1548, #1550, #1556).
