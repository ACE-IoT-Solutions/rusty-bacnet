# MCP server (`bacnet-mcp`, `bacnet mcp serve`)

A read-only [Model Context Protocol](https://modelcontextprotocol.io) server
for BACnet sites (#2066), phase 1 of the MCP epic (#2064). The design, with
the later phases, is [design/mcp.md](design/mcp.md); the user guide is the
website's [AI agents](https://jscott3201.github.io/rusty-bacnet/guides/ai-agents/)
page. Experimental in 0.15.

## Pieces

| Piece | Where | What it does |
|---|---|---|
| `bacnet-mcp` | `crates/bacnet-mcp` | `McpServer<T>`: the tool surface, prompts, policy, untrusted-text envelope, signed cursors. Out of `default-members`; publishable, because the CLI depends on it. Default feature `stdio`. |
| Engine extensions | `bacnet-ops` | `RequestLimits::with_network_rates`, `turn_on` and `slot`; per-caller `Account` and `TokenBucket` through `limits::charged`; `Engine::who_is`, `devices`, `device`, `address_style`; `read_trend` charged per request |
| `bacnet mcp` | `crates/bacnet-cli/src/mcp.rs` | `serve --stdio [--profile NAME] [--passive]` and `authorize --profile NAME`, behind the CLI feature `mcp` (which brings `tui`, for the profiles). Classified `Read` in `bacnet_ops::class`. |
| `[mcp]` profile table | `crates/bacnet-cli/src/tui/profile/mcp.rs` | networks, devices, slow and fast networks, broad discovery, rates and caps |

The SDK is `rmcp` 3.5.1 (pinned, server role, stdio only). It answers both
the 2026-07-28 request form (`server/discover`, version and capabilities in
each request's `_meta`) and the older `initialize` handshake.

## Tools

All are read-only (`readOnlyHint: true`, `destructiveHint: false`), with
input and output schemas and structured content plus a text rendering.

| Tool | Notes |
|---|---|
| `bacnet_discover` | Who-Is (local, network, directed, global), then the discovery table, paged (pages index the live table, which can change between them). Broad (global, no range, or more than 1000 instances) needs `broad_discovery`, one a minute; with `[mcp] devices` the range must lie inside them. A network-scope Who-Is takes that network's slot. The one tool in passive mode, where it only lists (by default) and refuses to send. |
| `bacnet_read` | RPM with the engine's RP fallback; array elements by RP; `preset` `point` or `device`; at most `max_values` rows a call, then a cursor |
| `bacnet_list_objects` | Object_List, cached five minutes per device for paging; `types` filter; `names` reads each name (50 a page). A list read element by element stops at the call's budget and keeps its place, so the next call (with `next_cursor`) goes on from there; `list.complete` says when it's done |
| `bacnet_describe_device` | The Device object's summary properties, services and object types by name, the address and route; `count_objects` |

Prompts: `survey_site` (range, network) and `investigate_point` (device,
object). Their arguments are checked against the target grammar.

The other five tools of phase 1 (`bacnet_find_object`, `bacnet_network_map`,
`bacnet_read_trend`, `bacnet_alarms`, `bacnet_decode`) follow in a second PR.

## Results

- Every result carries `cost` (`requests`, `elapsed_ms`, `throttled_ms`,
  `timeouts`) and, for success, `limits`; paged results carry `truncated`
  and `next_cursor`.
- Errors are `isError: true` results whose `error` says what happened:
  `invalid-argument`, `refused` (with `reason`, and `retry_after_s` when
  rate-limited) and `unknown-device` sent nothing; `bacnet` (the device's
  Error, Reject or Abort, kebab-case names with their numbers), `timeout`
  and `failed` did. An unknown tool or arguments outside the schema are
  JSON-RPC errors (-32602), and so is a tool this configuration doesn't
  list (a passive server's read tools). A test checks every tool's
  results, success and error, against its output schema. A cancelled call
  (`notifications/cancelled`) drops its future, so it sends nothing more.
- Values are `{display, typed}`: the shared formatter's text, and a typed
  form with enumeration names, named bits and dates and times by field. Raw
  hex only for octets that don't decode.
- Device text (names, descriptions, vendor strings, character-string
  values, and display text holding them) is an envelope
  `{untrusted, source, truncated}`: C0 and C1 controls and the line and
  paragraph separators become spaces; format characters (category Cf:
  bidirectional, zero-width, tag characters and the rest), variation
  selectors and Hangul fillers are dropped; `http://` becomes
  `hxxp://` (other schemes `scheme[:]//`), and the text is capped (256
  characters by default).
- Cursors are `hex(body).hex(HMAC-SHA256)` under a key drawn per process,
  binding the tool, a digest of the principal (the OS user's name doesn't
  show), a digest of the other arguments, an expiry (10 minutes) and the
  position.
- Display text holding any character string is enveloped whole, so a
  constructed value with a string inside comes back enveloped even where
  most of it isn't device text. Enveloping only the string leaves waits
  for the shared typed value form (#2092).

## Limits

Every request goes through `RequestLimits`: the caller's token first (5 a
second, burst 20), then the device's gate (one outstanding request), then
its network's rate (20, or 2 for `slow_networks` and 5 for
`fast_networks`; a router doesn't say what link it reaches), then the
global rate (20). Each tool call runs on its own task under an `Account`
that counts its requests and waits. A call that pages stops at its budget
(30 s) and returns a cursor; one still waiting `budget + 15 s` later
returns `refused: rate-limited` if it sent nothing, else `timeout`.

## Operation

- Stdout carries only JSON-RPC. `mcp.rs` denies `print_stdout` and
  `print_stderr` though the CLI allows them, tracing goes to stderr, and
  `crates/bacnet-cli/tests/mcp_stdio.rs` checks every stdout line.
- Live traffic needs a profile (maintainer decision, 2026-10-25). Without
  one, `serve` runs only with `--passive`; with one, transport flags are
  refused, since they would change the transport that was authorized.
- A profile serves MCP only once `bacnet mcp authorize` (a person typing
  the profile's name in a terminal) records, in the `[mcp]` table,
  `authorized_at_unix`, the table it covered as `[mcp.authorized]`, and the
  profile's transport settings as `authorized_transport`. A time without
  both authorizes nothing. Loading a profile whose table now does more
  (more networks or devices, broad discovery, a higher rate or cap, a
  network no longer slow or fast) or whose transport changed clears it.
- Reads are logged on stderr as summaries, always: one `read call` line
  per call (tool, device, requests, outcome) and `read summary` counts per
  device at shutdown. The `bacnet_mcp::reads` target stays on at info
  whatever `-v` or `RUST_LOG` say; they set the rest. They are not written
  to the TUI's `audit.jsonl`.
- `--passive` builds no client; only `bacnet_discover` is listed, and it
  refuses to send.

## Tests

- `crates/bacnet-mcp/src/tests/`: the stdio protocol over a pipe against
  an in-process lab (handshake, tool list, survey, errors, cursors, the
  2026-07-28 request form), no write tool in any configuration, passive
  mode, 500 reads within the global cap, one request outstanding per slow
  device, a slow network held to 2 a second, the injection fixture, the
  policy's reach, broad discovery (wide ranges, ranges outside the policy,
  parallel calls), network numbers, network-paced Who-Is, cancellation, a
  long indexed Object_List across calls, and output schemas.
- `crates/bacnet-ops`: network rates, accounts and buckets, a trend page
  charged per request, Who-Is through the engine.
- `crates/bacnet-cli`: the `[mcp]` table, authorization and its
  revocation, and the stdio hygiene test against the real binary.
