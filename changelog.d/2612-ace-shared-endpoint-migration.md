---
section: Migration notes
---
- **Breaking (ACE Python API, #2612).** Use `BACnetClient` and `BACnetServer` handles from the endpoint, strings for direct addresses, and `RoutedTarget(network, address, router=next_hop)`. For captured frames use `monotonic` for elapsed time, `timestamp` for POSIX seconds, and `missed_before` for the per-frame gap.
