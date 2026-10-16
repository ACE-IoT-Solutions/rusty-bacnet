---
section: Added
---
- Failed outbound SC TLS handshakes expose owned, explicitly untrusted peer
  certificates through Rust connect errors and Python transport exceptions.
  Capture uses the actual attempt and preserves normal certificate verification,
  rejection details and TLS resumption (#1735).
