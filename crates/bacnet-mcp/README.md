# bacnet-mcp

A read-only [Model Context Protocol](https://modelcontextprotocol.io) server
for BACnet sites, in the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace. An AI
agent host (Claude Code, Codex and others) launches it as `bacnet mcp serve
--stdio` (the `bacnet-cli` feature `mcp`) and calls its tools to discover
devices, describe them, list their objects and read their properties.

- Built on the official `rmcp` SDK; targets the 2026-07-28 revision of MCP
  and still answers the older `initialize` handshake.
- Every request goes through the `bacnet-ops` engine and its limits: one
  request at a time per device, a global rate, a rate per network and a
  token bucket per principal. Each result reports what it cost.
- Device-supplied text comes back in an untrusted envelope: cleaned of
  control and bidirectional characters, defanged and capped.
- Results are paged with cursors signed by a per-process key.
- Passive mode builds no client and sends nothing.

No tool writes. Experimental in 0.15; the API (`McpServer`, `Policy`)
changes as later phases add resources and guarded writes. The design is
[`docs/design/mcp.md`](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/design/mcp.md).
