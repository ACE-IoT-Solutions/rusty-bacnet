---
section: Migration notes
---
- **Owned NPDU (Rust API, #1770):** `bacnet_encoding::npdu::Npdu` drops
  `is_network_message` (call `is_network_message()`), its `message_type` is
  an `Option<NetworkMessageType>` and `NpduDecodeError::Malformed` carries a
  `bacnet_npdu::Malformed`; `ControlAuthContext::message_type` is a
  `NetworkMessageType`, and `router_table` no longer re-exports `Mac` or
  `RejectReason`.
