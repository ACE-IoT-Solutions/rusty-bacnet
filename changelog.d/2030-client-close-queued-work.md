---
section: Fixed
---
- **Python API:** A client call or entry queued behind a pending `async with`
  entry now fails with "client closing" as soon as `stop()` is called, instead
  of waiting for that entry to finish (#2030).
