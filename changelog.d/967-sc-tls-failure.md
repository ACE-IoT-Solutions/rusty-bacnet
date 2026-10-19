---
section: Changed
---
- **Rust API:** BACnet/SC connect errors carry the TLS library's reason
  (`ScConnectError::tls_failure`), and a classified handshake failure now
  reports `ScDisconnectCause::Tls` instead of `WebSocket(TlsHandshake)`. A host
  that does not resolve reports `ScWebSocketErrorKind::HostLookup` (#967).
