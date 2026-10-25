---
section: Changed
---
- The router's opt-in duplicate suppression also drops repeats of relayed
  I-Am-Router messages, and of Router-Busy and Router-Available from the
  same sender, which otherwise circulate for good around a loop of routers
  (#1810).
