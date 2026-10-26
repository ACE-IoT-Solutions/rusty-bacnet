---
section: Security
---
- The CLI and TUI drop bidirectional overrides, isolates, zero-width and
  other invisible format characters from device names and state texts, so a
  device string can no longer reorder or hide the rest of a row (#2122).
