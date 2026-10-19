---
section: Changed
---
- **Breaking (CLI):** commands that change a device need `--allow-write`,
  and DCC and reinitialize also `--yes` or a typed confirmation; a refusal
  exits with status 4 and sends nothing. `subscribe` defaults to 300
  seconds, renewed and cancelled on exit (#972).
