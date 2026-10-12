---
section: Changed
---
- **Breaking (Rust API):** `MstpConfig` gains the `extended_frames` field,
  and `TransportPort` the `prepare_for_routing` hook that `BACnetRouter`
  calls before it starts each port (#1611).
