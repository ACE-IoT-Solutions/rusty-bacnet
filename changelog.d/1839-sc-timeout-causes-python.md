---
section: Changed
---
- **Breaking (Python API):** An SC event's `cause` splits `connect_timeout`
  into `dial_timeout` (with `dial_stage`) and `connect_accept_timeout`, and
  adds `upgrade_refused` with `http_status` (#1839).
