---
section: Fixed
---
- **Wire:** BACnet/SC direct connections now take part in heartbeats: an
  accepted connection answers Heartbeat-Request, and a dialed one sends a
  heartbeat when nothing has been received for the interval and closes the
  link when it goes unanswered (#2127).
