---
section: Changed
---
- **Breaking (Rust API):** SC connect failures tell a stalled dial, with its
  step, from a hub that never sends Connect-Accept, and keep the HTTP status
  of a refused WebSocket upgrade; the TUI doctor places each (#1839).
