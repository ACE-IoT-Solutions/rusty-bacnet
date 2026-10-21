---
section: Migration notes
---
- **Network layer replies (Rust API, #1699):** a `ReceivedApdu` built by hand
  sets the new `router_response: None` (a router fills it in only for a
  Confirmed-Request it delivers locally); keep `response_route()` for router
  deliveries. The `*_on_issuance` callbacks must own their captures
  (`Send + 'static`), since a router queue may hold them until the send.
