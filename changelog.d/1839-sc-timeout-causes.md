---
section: Changed
---
- **Breaking (Rust API):** A BACnet/SC dial that runs out of time fails with
  `ScConnectError::DialTimeout`, naming the stalled step, inside
  `Error::Transport` rather than `Error::Timeout`, and a refused WebSocket
  upgrade keeps its HTTP status (#1839).
