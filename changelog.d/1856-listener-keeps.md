---
section: Changed
---
- `bacnet-bip-core` exposes `Link::listener_keeps`, the node's broadcast-listener verdict, and the B/IP transport's frame tap uses it, so a foreign device's listener no longer taps a Forwarded-NPDU its node refuses (#1856).
