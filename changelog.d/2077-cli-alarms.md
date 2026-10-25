---
section: Changed
---
- **Breaking (CLI output):** `bacnet alarms` prints a decoded table, or JSON, of each
  object's event state, transitions, timestamps, notify type and priorities, and
  follows the device's pages up to `--max`, instead of hex (#2077).
