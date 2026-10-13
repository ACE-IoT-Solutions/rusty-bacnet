---
section: Fixed
---
- **Wire:** At startup `BACnetRouter` broadcasts a Network-Number-Is on each
  port, giving the port's configured network number, before its
  I-Am-Router-To-Network; a one-port router now sends that one message
  (#1679).
