---
section: Fixed
---
- MS/TP: after a frame is aborted part way out, the master core counts the
  line's silence from the abort, so it no longer generates a token before
  its own slot (#1640).
