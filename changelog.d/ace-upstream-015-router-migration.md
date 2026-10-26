---
section: Migration notes
---
- **ACE mixed router:** Replace `BACnetRouter.from_ports(...)` with `AceMixedRouter.from_ports(...)` and import `AceMixedRouter`. Existing `RouterBipPort` and `RouterScPort` configurations, `start`/`stop`, health and routing snapshots remain available. Upstream `BACnetRouter` uses `BipRouterPort` and `close`.
