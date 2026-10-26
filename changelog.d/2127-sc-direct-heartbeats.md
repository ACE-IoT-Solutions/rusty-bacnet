---
section: Fixed
---
- **Wire:** BACnet/SC direct connections now take part in heartbeats: an
  accepted connection answers Heartbeat-Request, and a dialed one probes an
  idle peer at the configured interval and closes the link when the probe
  goes unanswered (#2127).
