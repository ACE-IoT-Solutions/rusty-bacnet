---
section: Changed
---
- **Wire:** The MCP server's watch hub polls each device's polled points together, in batched ReadPropertyMultiple requests, one request at a time per device; a retried subscription reads the point again first (#2115).
