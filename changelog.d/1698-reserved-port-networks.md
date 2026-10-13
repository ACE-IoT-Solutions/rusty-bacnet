---
section: Changed
---
- **Breaking (Rust API):** `BACnetRouter::start` and the router core's
  `Router::new` refuse a port on network 0 or 0xFFFF, which they used to run
  with no direct route; number each port from 1 to 65534 (#1698).
