---
section: Added
---
- `bacnet tui` shows device and object names, reading only the rows on
  screen within `--max-rate` in batches sized to the device's Max_APDU, and
  keeps up to 4,096 per device address; `/` matches names and `s` sorts the
  device table by them (#1896, #1929, #1930, #1931).
