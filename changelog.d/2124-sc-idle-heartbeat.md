---
section: Fixed
---
- An idle BACnet/SC device no longer drops a hub that answers its heartbeats,
  sends a heartbeat every interval instead of every second one, and drops an
  unanswered link one timeout after the last frame received, sooner than
  before (#2124).
