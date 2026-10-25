---
section: Fixed
---
- **Python API:** DCC and ReinitializeDevice requests with a password over
  20 characters fail before sending: the encoders return an error, the CLI
  `control` and `reinit` refuse it at parse time, and the Python client
  raises ValueError (#2085).
