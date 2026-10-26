---
section: Changed
---
- `RouterPort` and `EndpointSession` default their transport type to `AnyTransport`, as `BACnetClient` and `BACnetServer` do, so a mixed-transport router or endpoint can leave it out (#2043).
