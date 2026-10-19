---
section: Migration notes
---
- **Shell history (CLI, #962):** to keep earlier history, move
  `~/.bacnet_history` to `shell_history` in the data directory:
  `~/.local/share/bacnet/` (or `$XDG_DATA_HOME/bacnet/`) on Linux,
  `~/Library/Application Support/bacnet/` on macOS, `%APPDATA%\bacnet\data\`
  on Windows. See [the shell](docs/CLI.md#interactive-shell).
