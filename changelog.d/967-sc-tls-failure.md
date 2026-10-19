---
section: Added
---
- **Rust API:** BACnet/SC connect errors carry a typed TLS reason
  (`ScConnectError::tls_failure`, `ScDisconnectCause::Tls`), and a host name
  that does not resolve reports `ScWebSocketErrorKind::HostLookup` rather than
  a TCP dial failure (#967).
