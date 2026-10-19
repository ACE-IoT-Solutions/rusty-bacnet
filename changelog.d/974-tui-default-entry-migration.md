---
section: Migration notes
---
- **CLI (#974):** `bacnet shell` is gone: open the TUI (`bacnet` or `bacnet
  tui`) or use the one-shot commands, with `--json` for scripts and screen
  readers. A script that piped commands into the shell calls one one-shot per
  command instead (default targets become explicit). For `devices`, use
  `discover` (or `discover --json`) or the TUI's device list.
  `--no-default-features` builds the CLI without the TUI.
