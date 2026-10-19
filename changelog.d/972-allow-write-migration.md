---
section: Migration notes
---
- **CLI write guard (#972):** add `--allow-write` to scripts that run `write`,
  `create-object`, `delete-object`, `file-write`, `ack-alarm`, `time-sync` or
  `unregister`, and `--allow-write --yes` for `control` (`dcc`) or `reinit`.
  A refusal exits with status 4. An indefinite `subscribe` needs
  `--lifetime 0 --allow-write`. See [the write guard](docs/CLI.md#write-guard).
