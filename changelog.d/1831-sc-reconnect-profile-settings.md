---
section: Added
---
- TUI SC profiles reconnect with backoff by default, primary-only ones too, and
  can set the backoff, the connect timeout or `reconnect = false`; new
  `--sc-failover-url`, `--sc-reconnect` and `--sc-connect-timeout` flags give
  flags-only runs the same (#1831).
