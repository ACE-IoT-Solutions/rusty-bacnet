---
section: Migration notes
---
- **Router startup (wire, #1679):** Each `BACnetRouter` port now sends one
  more message at startup, a Network-Number-Is, ahead of its
  I-Am-Router-To-Network. A test or tool that reads a port's first message
  as the I-Am-Router should skip it, and devices on the port learn its
  network number from it.
