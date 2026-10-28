---
section: Added
---
- The MCP device list pages past its size cap: a truncated
  `bacnet://devices` names the next page in `next`, and
  `bacnet://devices/after/{instance}` lists the devices above an instance,
  so pages hold steady while discovery adds devices (#2116).
