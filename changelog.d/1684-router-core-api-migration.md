---
section: Migration notes
---
- **Router core (Rust API, #1684):** `Fixes` and `Options::fixes` are
  gone. Renamed: `DropReason` to `Discard` (`Received::Discarded`),
  `table::Refusal` to `LearnRefusal`, `NoRoom`'s `capacity` to
  `available`, `parse_disconnect` to `DisconnectConnection::parse`; message
  views return `Result`. Gone: `Discovery::COOLDOWN`/`MAX_AGE` (in
  `Options::HOSTED`), `Table::new_hardened` (`set_convergence`). Matches on
  errors, `Learned` or `ControlClass` need `_`
  ([all of it](docs/rust-api.md#bacnet-router-core-firmware)).
