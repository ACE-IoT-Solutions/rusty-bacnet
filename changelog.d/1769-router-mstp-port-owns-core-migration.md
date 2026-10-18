---
section: Migration notes
---
- **Router MS/TP port (Rust API, #1769):** build it with
  `MstpPort::new(port, core)?`, take its `PortConfig` from `port_config`,
  poll `poll_action()` (NPDUs arrive as `PortAction::Arrived`), call
  `release` and `send` on the port, and reach the core through `core_mut()`.
