---
section: Changed
---
- A B/IP transport sharing its port by address hands up broadcasts and
  unicast in the order the kernel received them on Linux, macOS and the BSDs,
  so a unicast that depends on a broadcast just before it is handled after it
  (#1546).
