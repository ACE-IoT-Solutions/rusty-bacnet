---
section: Added
---
- **Wire:** `MstpConfig::extended_frames` (off by default) lets the MS/TP
  transport send and take NPDUs of up to 1497 octets, APDUs of 1476, in
  extended frames, and `BACnetRouter` turns it on for every MS/TP port it
  routes, as Clause 9.3 requires of routers (#1611).
