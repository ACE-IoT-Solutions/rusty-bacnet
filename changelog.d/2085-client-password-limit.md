---
section: Fixed
---
- DCC and ReinitializeDevice requests with a password over 20 characters
  fail before sending: `encode` now returns an error for a password over 20
  characters, the CLI `control` and `reinit` refuse it at parse time, and
  the Python client raises ValueError (#2085).
