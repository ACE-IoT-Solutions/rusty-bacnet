---
section: Added
---
- A ReinitializeDevice handler can register its restart with
  `ReinitializeContext::after_reply`, which the server and endpoint sessions
  run once the SimpleACK has been handed to the transport, and never for a
  request answered with an Error (#1565).
