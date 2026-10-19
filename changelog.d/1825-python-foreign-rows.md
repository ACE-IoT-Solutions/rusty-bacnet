---
section: Changed
---
- **Python API:** With `on_reinitialize` set, a Python foreign device's
  FD_BBMD_Address and FD_Subscription_Lifetime accept writes, applied by
  ACTIVATE_CHANGES or WARMSTART, where they answered WRITE_ACCESS_DENIED
  (#1825).
