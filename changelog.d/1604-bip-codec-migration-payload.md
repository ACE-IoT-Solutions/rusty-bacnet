---
section: Migration notes
---
- **BvllMessage (Rust API, #1604):** `payload` is now everything after the
  header, so a Forwarded-NPDU's starts with the 6-octet origin: code that
  read the NPDU from `.payload` still compiles but gets 6 extra octets; use
  `npdu()`. `originating_ip`/`originating_port` become `origin()`.
  `decode_bvll` refuses a datagram whose Length field isn't its length.
