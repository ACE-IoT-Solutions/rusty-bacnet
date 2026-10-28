---
section: Added
---
- `bacnet read-range` and the MCP `bacnet_read_trend` tool name a Trend Log's
  logged enumerations and bit strings by the property its
  Log_DeviceObjectProperty names, at the cost of one extra read; values print
  as before when that read fails (#2091).
