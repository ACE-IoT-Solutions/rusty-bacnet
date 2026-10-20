---
section: Migration notes
---
- **CLI output (#1916):** a script reading `discover --json` takes the routed
  station from `station` (was `remote_mac`), and one reading `subscribe --json`
  takes it from `routed_source` (was `source_address`). Both hold `NET:0xHEX`,
  network included; the table columns read the same way.
