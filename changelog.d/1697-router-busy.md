---
section: Added
---
- Routers can announce Router-Busy for a port whose egress queue stays full, reject what that queue then refuses with reason 2, and announce Router-Available once it drains: off by default, turned on with `RouterOptions::congestion` or the core's `Options::congestion` (#1697).
