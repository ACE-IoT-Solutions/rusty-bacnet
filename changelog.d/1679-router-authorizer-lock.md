---
section: Changed
---
- A router's control authorizer now runs under the router's engine lock,
  one call at a time, where it could run concurrently before. One that
  keeps to its contract (no blocking, no calls into the router) sees no
  difference (#1679).
