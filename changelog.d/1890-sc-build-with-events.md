---
section: Added
---
- **Rust API:** `ScClientBuilder::build_with_events` and
  `build_any_with_events` build a BACnet/SC client, concrete or
  transport-erased, with a hub-connector event receiver that already holds
  the first `Connected` (#1890, #1909).
