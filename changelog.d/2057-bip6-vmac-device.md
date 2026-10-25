---
section: Changed
---
- **Breaking (Rust API):** A server or endpoint refuses to start a registered
  B/IPv6 Network Port whose transport VMAC isn't the served Device's
  instance, so the port never publishes another Device's VMAC (#2057).
