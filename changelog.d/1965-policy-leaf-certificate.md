---
section: Security
---
- **Breaking (Rust API):** With `sc-tls`, server policy contexts expose the
  verified leaf certificate behind a direct BACnet/SC principal, with its subject
  and Subject Alternative Names, so rules by name survive certificate renewal
  (#1965).
