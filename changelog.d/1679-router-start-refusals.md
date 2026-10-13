---
section: Changed
---
- **Breaking (Rust API):** `BACnetRouter::start` refuses a router without
  ports, with more than 255, or with a port whose own MAC is longer than 18
  octets, which it started before (#1679).
