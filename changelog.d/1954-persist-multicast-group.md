---
section: Added
---
- A B/IP multicast group activated through the Network Port can be kept across a restart with `BipTransport::set_multicast_persist_path`; a start prefers the saved group over the configured one, and a failed save is logged and counted (#1954).
