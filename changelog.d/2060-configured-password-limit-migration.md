---
section: Migration notes
---
- **DCC and ReinitializeDevice passwords (#2060):** keep a configured
  `dcc_password` or `reinit_password` to 20 characters; a longer one now
  fails at start (in Python, at construction) instead of refusing every
  request.
