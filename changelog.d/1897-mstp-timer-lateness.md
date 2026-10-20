---
section: Changed
---
- **Breaking (Rust API):** MS/TP sorts timer lateness into a fixed histogram
  with its maximum, and `deadline_misses` becomes `late_deadlines`, counted
  from the new `MstpCoreConfig::late_deadline` (1 ms by default, 5 ms in the
  Tokio transport). The TUI shows both (#1897).
