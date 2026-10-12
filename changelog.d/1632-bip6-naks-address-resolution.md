---
section: Fixed
---
- **Wire:** A B/IPv6 node now NAKs the three requests only a BBMD serves
  (foreign-device registration, FDT entry deletion and broadcast
  distribution), and answers a BBMD's Forwarded-Address-Resolution for its
  VMAC, so address resolution across BBMDs reaches it. Both are rate-limited
  (#1632).
