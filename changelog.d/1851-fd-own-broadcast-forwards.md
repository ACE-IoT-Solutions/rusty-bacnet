---
section: Fixed
---
- A B/IP foreign device whose broadcast address is its own address, as in
  loopback tests, now takes the Forwarded-NPDUs its BBMD sends it instead of
  dropping them as untrusted broadcasts (#1851).
