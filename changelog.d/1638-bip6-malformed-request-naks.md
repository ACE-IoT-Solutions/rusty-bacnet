---
section: Fixed
---
- **Wire:** A B/IPv6 node now NAKs a malformed foreign-device registration,
  FDT entry deletion or broadcast distribution as it does a well-formed one,
  under the same rules and rate limit, as B/IP does (#1638).
