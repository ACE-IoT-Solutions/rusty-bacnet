---
section: Added
---
- **Wire:** A registered foreign device's Network Port takes writes to
  FD_BBMD_Address and FD_Subscription_Lifetime, pending until ReinitializeDevice
  ACTIVATE_CHANGES or WARMSTART, which moves its next registration to the new
  BBMD and time-to-live (#1825).
