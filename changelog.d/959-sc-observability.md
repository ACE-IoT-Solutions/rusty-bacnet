---
section: Added
---
- BACnet/SC hub-connector observability: `connection_events()` reports
  connects, disconnect causes, reconnects, failover and primary restore
  attempts, `connection_status()` gives the active hub, hub VMAC and
  heartbeat timing, and `ScClientBuilder::failover_hub_url` sets a failover
  hub (#959, #1828).
