---
section: Changed
---
- **Breaking (CLI):** Plain `bacnet` on a terminal opens the TUI, now a default
  feature; elsewhere, or with `--no-tui` or `BACNET_NO_TUI`, it prints the
  usage and exits 2. `bacnet shell` and rustyline are removed, and the
  shell's `writem` is a one-shot command (#974).
