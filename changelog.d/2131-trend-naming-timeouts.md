---
section: Fixed
---
- The MCP `bacnet_read_trend` tool counts a timeout while naming a log's
  values or reading a later page, so a device silent after page 1 shows
  it; `bacnet_read` and `bacnet_find_object` no longer count a transport
  failure as a timeout (#2131).
