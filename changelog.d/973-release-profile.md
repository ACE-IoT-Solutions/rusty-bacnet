---
section: Changed
---
- The published `bacnet` CLIs and Python wheels are built with whole-program
  LTO in a new `dist` profile, so they are about a third smaller (the macOS
  arm64 CLI from 18.9 to 12.4 MB); panic backtraces keep their function names
  (#973).
