---
section: Changed
---
- **Breaking (Rust API):** a Reject-Message-To-Network's reason and network
  are read and written in one place, `bacnet_npdu::RejectMessageToNetwork`,
  which the router core, network layer and client share (#1794).
