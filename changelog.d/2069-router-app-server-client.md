---
section: Added
---
- `StartedRouter::into_application_ports` splits the router's own application between a `BACnetServer` and a `BACnetClient`: requests reach the server, and each answer reaches the stack that asked (#2069).
