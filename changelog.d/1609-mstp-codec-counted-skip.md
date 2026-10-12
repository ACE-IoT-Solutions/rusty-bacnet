---
section: Fixed
---
- The MS/TP stream decoder skips a frame it drops after a good header by
  count, reporting its whole length, so no false frame comes out of its
  body, and it discards a run of line noise in one event (#1609).
