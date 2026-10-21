---
section: Changed
---
- **Rust API:** BACnet/SC connect errors carry the TLS library's reason
  (`ScConnectError::tls_failure`), and a host that does not resolve reports
  `ScWebSocketErrorKind::HostLookup` (#967).
