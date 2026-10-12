---
section: Changed
---
- `BACnetRouter` warns about a port's failed sends at most once every 10 s,
  with the count since the last warning, instead of once per frame;
  `bacnet_transport::warn_throttle::WarnThrottle` is public (#1672).
