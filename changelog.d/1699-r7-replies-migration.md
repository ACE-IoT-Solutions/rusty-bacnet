---
section: Migration notes
---
- Manual `ReceivedApdu` construction needs `router_response: None`; preserve `response_route()` for router deliveries. Response issuance callbacks now require owned `Send + 'static` captures because a router queue may retain them until selection (#1699).
