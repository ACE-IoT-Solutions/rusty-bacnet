---
section: Added
---
- **MS/TP endpoint:** `MstpEndpointBuilder::extended_frames` turns on
  extended frames for an endpoint's link and lets its identity advertise a
  Max_APDU_Length_Accepted of up to 1476 instead of 480; the default is
  unchanged (#1663).
