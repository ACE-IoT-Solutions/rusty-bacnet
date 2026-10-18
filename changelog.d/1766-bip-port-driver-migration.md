---
section: Migration notes
---
- **B/IP port driver (Rust API, #1766):** use `bacnet_bip_core::port`:
  `NodePort::new(node, held)`, `BbmdPort::new(link, tables, targets, rows)`,
  with `HeldBroadcast`, `Datagrams` (refusing with `SendRefusal::SocketFull`
  or `SocketError`), `Ingress` and `PortError` there; route through
  `bacnet_router_core::bip::BipPort` (`config`, `arrived`, `send`).
  `Ingress::Management` and `BbmdPort::on_deadline` are gone; management
  requests are `ForBbmd::Management(Request)`.
