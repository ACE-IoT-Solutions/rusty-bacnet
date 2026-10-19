---
section: Added
---
- BACnet/SC hub-connector observability (#959): `connection_events()` reports
  connects, disconnect causes, reconnects, failover and primary restore,
  `connection_status()` gives the active hub, hub VMAC and heartbeat timing,
  and `ScClientBuilder::failover_hub_url` sets a failover hub.
