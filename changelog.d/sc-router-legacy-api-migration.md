---
section: Migration notes
---
- **Router (Python API):** The fork-only `BACnetRouter(bip_network, virtual_ports, ...)`, `RouterVirtualPort`, and forwarding counters are removed. Use `BACnetRouter.from_ports()` with typed B/IP or SC ports, `port_health()`, and `routing_table()`.
