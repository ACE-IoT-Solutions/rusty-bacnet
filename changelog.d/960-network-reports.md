---
section: Added
---
- `NetworkLayer::send_network_message` sends Who-Is-Router-To-Network and
  What-Is-Network-Number, and `ReceivedNetworkReport::parse` reads the reports
  a non-routing node acts on; `bacnet_router_core::table::is_reserved` is
  public (#960).
