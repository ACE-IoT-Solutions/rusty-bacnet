---
section: Migration notes
---
- **Network-layer types (Rust API, #1770):** import `NetworkMessageType`,
  `RejectMessageReason`, `NetworkPriority` (now `Normal`, `Urgent`,
  `CriticalEquipment`, `LifeSafety`), `NpduLimits` and `MAX_MAC_LEN` from
  `bacnet_npdu`. Router core: `MessageType`, `RejectReason`, `Limits`,
  `Npdu::parse` and `write` become `NetworkMessageType`,
  `RejectMessageReason`, `NpduLimits`, `decode` and `encode_into`.
