---
section: Migration notes
---
- **CLI write guard (#972):** add `--allow-write` to scripts that run `write`,
  `create-object`, `delete-object`, `file-write`, `ack-alarm`, `time-sync` or
  `unregister`, and `--allow-write --yes` to scripts that run `control` (`dcc`)
  or `reinit`. Start the shell with `bacnet --allow-write shell` to write from
  it. Reads, `subscribe` and `register` are unchanged. With JSON output a
  refusal is one JSON line on stderr; see [the write guard](docs/CLI.md#write-guard).
