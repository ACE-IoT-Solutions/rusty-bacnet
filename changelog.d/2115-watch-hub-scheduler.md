---
section: Changed
---
- **Wire:** The MCP watch hub polls a device's points together in batched ReadPropertyMultiple requests, backs off per device, rereads a point before retrying its subscription; the TUI asks a too-large batch again a row at a time by RPM rather than ReadProperty (#2115).
