---
section: Fixed
---
- The MCP `bacnet_read_trend` tool counts a timeout while it names a log's
  values, or while it reads a later page, in its cost, so a device that went
  silent after the first page no longer reports no timeouts (#2131).
