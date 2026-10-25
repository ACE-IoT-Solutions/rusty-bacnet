---
title: "Use rusty-bacnet with AI agents"
description: "Let an AI agent host such as Claude Code discover and read a BACnet site through a read-only MCP server."
---

`bacnet mcp serve` is a [Model Context Protocol](https://modelcontextprotocol.io) server: an agent host such as Claude Code or Codex starts it as a child process and calls its tools to discover devices, describe them, list their objects and read their properties. It is experimental in 0.15, built only with the CLI's `mcp` feature, and **read-only**: no configuration offers a tool that writes.

## Build it

The release executables leave the feature out. Build the CLI from a source checkout with it:

```sh
cargo install --path crates/bacnet-cli --locked --features mcp
```

## Authorize a profile

The server connects through a [TUI profile](/rusty-bacnet/tui/profiles/), whose `[mcp]` table limits what an agent may reach. An absent table reaches every network and device, at the default rates.

```toml
[mcp]
networks = [0, 2001]          # 0 is the local network
devices = ["1000-1999", "5"]
slow_networks = [2001]        # an MS/TP trunk behind a router: 2 requests a second
broad_discovery = false       # true allows a global or unbounded Who-Is, one a minute
```

Before an agent host can use the profile, a person consents in a terminal. The command shows what the table lets an agent reach and asks for the profile's name:

```sh
bacnet mcp authorize --profile plant-a
```

Widening the table later (another network or device, or broad discovery) clears the authorization, so the server refuses to start until it is given again. Narrowing it doesn't.

## Add it to the agent host

Claude Code reads a project's `.mcp.json`. The host launches the command and speaks MCP on its stdin and stdout:

```json
{
  "mcpServers": {
    "bacnet": {
      "command": "bacnet",
      "args": ["mcp", "serve", "--stdio", "--profile", "plant-a"]
    }
  }
}
```

Without `--profile`, the global transport flags pick the network, as for a one-shot command. `--passive` builds no client and sends nothing; only the discovery table, empty then, is offered. Logs go to stderr (`-v` for more), never to stdout, which carries only the protocol.

```sh
bacnet mcp serve --stdio --profile plant-a --passive
```

## What the agent can do

| Tool | What it does |
|---|---|
| `bacnet_discover` | Sends a Who-Is (local, to one network, to one address, or global) and lists the devices that answered |
| `bacnet_describe_device` | Summarises a device: vendor, model, firmware, protocol revision, supported services and object types, and object counts |
| `bacnet_list_objects` | Lists a device's objects, filtered by type, with their names on request |
| `bacnet_read` | Reads properties, by ReadPropertyMultiple with a ReadProperty fallback; `preset: "point"` reads what matters about a point |

Two prompts start common jobs: `survey_site` surveys a range of devices, and `investigate_point` explains one point's value, status and priority array. Every value comes as display text and a typed form with enumeration and bit names. A property a device can't give is an error row with the device's error class and code.

## Safety

- **Device text is data.** Names, descriptions and vendor strings come back in an `untrusted` envelope that says where each came from, with control characters and bidirectional overrides removed, links defanged and the length capped. A device can still say anything in them, so a model can still be misled by what it reads; the server's answer is that it can't change anything.
- **Limits hold on the server.** One request at a time per device, 20 a second in all, 20 a second per network (2 on a network marked slow, 5 on one marked fast), and 5 a second for the agent, with bursts of 20. Each result reports what it cost, including the time spent waiting, so an agent can slow down.
- **Results are paged.** Large lists end with a signed cursor that only resumes the same call.
- **Reads are logged as summaries** on stderr: a line per call, and the counts per device when the server stops.
- **Site data reaches the agent's host.** Don't share an agent session that reads a sensitive site with tools that can send data elsewhere.

The design, including the write tools of a later phase, is in [`docs/design/mcp.md`](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/design/mcp.md).
