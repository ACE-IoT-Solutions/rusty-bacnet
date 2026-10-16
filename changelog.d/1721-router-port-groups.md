---
section: Fixed
---
- **Breaking (Rust API):** portable router ports now supply their group-address
  rules, so B/IP and MS/TP group forwarding works with the default host.
  Custom links keep their own classification rules (#1721).
