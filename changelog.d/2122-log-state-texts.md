---
section: Added
---
- `bacnet read-range` and the MCP `bacnet_read_trend` tool name a logged
  binary or multi-state state with the monitored object's own state texts, at
  the cost of one extra read an object; the MCP tool labels its state counts
  with them in untrusted envelopes (#2122).
