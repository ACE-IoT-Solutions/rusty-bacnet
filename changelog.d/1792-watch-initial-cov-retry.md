---
section: Fixed
---
- TUI watch rows now retry COV after an initial read or subscription times out, using the existing bounded backoff while polling. Removing a row or quitting preserves cancellation of a possible subscription across queued and in-flight retries (#1792).
