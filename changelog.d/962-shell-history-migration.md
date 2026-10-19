---
section: Migration notes
---
- **Shell history (CLI, #962):** the shell now keeps its history as
  `shell_history` in the platform data directory. On first start it copies
  `$HOME/.bacnet_history` there once, if that exists; a history the old
  release kept in the working directory (Windows without `HOME`) must be
  moved by hand. See [the shell](docs/CLI.md#interactive-shell).
