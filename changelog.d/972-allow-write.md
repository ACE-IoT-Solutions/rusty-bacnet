---
section: Changed
---
- **Breaking (CLI):** one-shot commands and the shell refuse anything that
  changes a device without `--allow-write`, and communication control and
  reinitialize also need `--yes` or a typed confirmation; a refused command
  exits with status 2 and sends nothing (#972).
