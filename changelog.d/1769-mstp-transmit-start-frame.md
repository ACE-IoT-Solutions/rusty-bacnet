---
section: Changed
---
- **Breaking (Rust API):** an MS/TP core's frame octets now come only from a
  confirmed `on_transmit_started`, so a refused start has nothing to send;
  `Action::Transmit` names the frame by header and length (#1769).
