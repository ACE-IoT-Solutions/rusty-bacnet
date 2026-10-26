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
| Engine extensions | `bacnet-ops` | `RequestLimits::with_network_rates`, `turn_on` and `slot`; per-caller `Account` and `TokenBucket` through `limits::charged`; `Engine::who_is`, `discover_routers`, `devices`, `device`, `address_style`; `read_trend` charged per request; `alarm_summary` and `enrollment_summary` |
| Decoder | `bacnet-ops::decode` | The capture screen's frame summary and field tree, moved from the CLI for `bacnet_decode`: any tapped link (B/IP, B/IPv6, BACnet/SC, MS/TP), or a bare NPDU or APDU; a field quoting a character string is marked as device text |
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
| `bacnet_discover` | Who-Is (local, network, directed, global), then the discovery table, paged (pages index the live table, which can change between them). Broad (global, no range, or more than 1000 instances; directed without a range to an address that isn't a known device) needs `broad_discovery`, one a minute, whatever the reach; with `[mcp] devices` the range must also lie inside them. A send the transport refuses isn't charged. A network-scope Who-Is takes that network's slot. The one tool in passive mode, where it only lists (by default) and refuses to send. |
| `bacnet_read` | RPM with the engine's RP fallback; array elements by RP; `preset` `point` or `device`; at most `max_values` rows a call, then a cursor |
| `bacnet_list_objects` | Object_List, cached five minutes per device for paging; `types` filter; `names` reads each name (50 a page). A list read element by element stops at the call's budget and keeps its place, so the next call (with `next_cursor`) goes on from there; `list.complete` says when it's done. Calls on one device take turns, so progress never goes back. A resumed read checks the count again and starts over if it changed, and one begun more than 10 minutes ago starts over; an element the device can't give is a gap (`list.gaps`); at most 64 devices' lists are kept |
| `bacnet_describe_device` | The Device object's summary properties, services and object types by name, the address and route; `count_objects` |
| `bacnet_find_object` | Objects across the discovery table, within reach and an optional device `range`: by `name` or `description` (a case-insensitive substring of the cleaned device text), `object` or `types`. Lists come from `bacnet_list_objects`' cache (read once, kept five minutes); a text match reads each candidate's name (and description, if asked) once, kept five minutes too. A page holds up to 100 matches (or the policy's object page), and a full page ends before the next device is read; the cursor packs a device and a candidate index. Names are kept cleaned and capped, at most 4,096 objects a device. A device whose list read stopped is under `incomplete` with why, after searching what was read; one that failed is under `skipped` with its error; a list read cut by the budget resumes on the next page. A device that stops answering while its names are read is under `incomplete` too, and the search moves on without a cursor back to it, so a silent device doesn't cost each later page a timeout; search it alone to retry. |
| `bacnet_network_map` | Networks in `[mcp] networks`, each with its routers (from the last Who-Is-Router-To-Network and from the routed devices' router addresses), its speed and rate under the policy, and its devices in reach; and the routers with the networks each reaches. `send: true` broadcasts a Who-Is-Router-To-Network: it needs `broad_discovery`, goes at most once a minute (checked and claimed under one lock), takes the local network's slot and is charged. The wait for answers ends with the call's budget. A query cancelled while it waits keeps its claim on the minute but records no routers; at most 256 routers are kept, replaced by each finished query (one asking about a single network merges into them). No network-layer writes. |
| `bacnet_read_trend` | A Trend Log or Trend Log Multiple through `Engine::read_trend`, each request charged. From the oldest record or after `from`, up to `to`; at most `max_records` (500 by default, 2,000 at most, and about 60 kB of structured content) a call, then a cursor holding the log's next sequence number. `from` after `to` is a JSON-RPC -32602. Records: time, sequence number, `values` as `Datum`s (a failure as its error), status flags named, `flagged` when a status flag is set or the record is a log status change. A summary of the records read comes with every call: count, flagged, log status and time-change counts, first and last time, and per column (a member of a Trend Log Multiple) count, min, max and mean of numbers, counts of booleans and enumerations by value, failures. `summary: true` reads up to 20,000 records and returns only the summary. `pages`, `more_items` (answers cut short by the device) and `gaps` report how the reading went. |
| `bacnet_alarms` | GetEventInformation through `bacnet_ops::format::alarms` (#2077): objects, states, transitions, notify types, timestamps and priorities named; paged over the device's answers (`more_events`) up to `max` (100) a call, the cursor holding the last object; once the call's budget is spent no further answer is asked for, and what was read comes back with `more_events` and a cursor (`format::alarms::read_event_information_until`).
 `summaries: true` adds GetAlarmSummary and GetEnrollmentSummary on the first page; a device refusing one reports it under `summary_errors`. Every datum is passed through the envelope check. |
| `bacnet_decode` | Hex (separators and `0x` ignored, at most 6,144 octets, room for the largest BACnet/SC message the stack accepts) as `bip`, `bip6`, `mstp`, `sc`, `npdu`, `apdu` or `auto` (by the first octets; SC must be named). Returns a one-line `text`, the decoder's `summary` and the field `tree` (labels with `[start, end)` octet ranges, at most 600 fields). A label quoting a character string is an envelope with source `decoded frame`; other labels are cleaned, and a long one (an unknown tag's octets) is cut at 512 characters with `truncated: true`. No traffic: listed in passive mode too. |

Prompts: `survey_site` (range, network) and `investigate_point` (device,
object). Their arguments are checked against the target grammar. The survey
also maps the networks and lists each device's alarms; the point
investigation also checks its alarms and summarises a trend log of it.

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
- Values are `{display, typed}`: the shared formatter's text, and its
  typed form, `bacnet_ops::format::Datum`, the shape the CLI's JSON uses
  (numbers as numbers, `{value, name}` for enumerations, `{bits, names}` for
  bit strings, `{type, type_number, instance}` for objects, `{element,
  text}` for constructed elements). Raw hex only for octets that don't
  decode.
- Device text is an envelope, leaf by leaf: names, descriptions, vendor
  strings, a character-string value in `typed`, and the `text` of a
  constructed element whose kind can hold a string (a name and value, a
  schedule's values, an access result, an unnamed kind), but not of one
  that can't (a time stamp, a reference, a date range). The display text is
  enveloped whole only when the value holds device text. Each envelope is
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
- `--passive` builds no client; only `bacnet_discover` (which refuses to
  send) and `bacnet_decode` are listed.

## Tests

- `crates/bacnet-mcp/src/tests/`: the stdio protocol over a pipe against
  an in-process lab (handshake, tool list, survey, errors, cursors, the
  2026-07-28 request form), no write tool in any configuration, passive
  mode, 500 reads within the global cap, one request outstanding per slow
  device, a slow network held to 2 a second, the injection fixture, the
  policy's reach, broad discovery (wide ranges, ranges outside the policy,
  parallel calls), network numbers, network-paced Who-Is, cancellation, a
  long indexed Object_List across calls, and output schemas (success and
  error results of all nine tools, nested values included).
- `crates/bacnet-mcp/src/tests/site.rs` and `trends.rs` (part B): finding
  objects by name, type and object, cached names, pages across devices,
  reach and skipped devices; the network map, its broad-request rule and
  its reach; alarms paged with summaries; the decode injection fixture in
  passive mode; trend paging across ReadRange answers cut short and across
  calls, the summary against a brute-force computation (acceptance 7), time
  windows, states, a spent budget resuming without loss, and cancellation.
  `limits.rs`: a device silent mid-list or while naming, a list cut by the
  budget, alarms stopped by the budget, the router wait ending with the
  budget, a cancelled router query, and a backwards trend window.
- `crates/bacnet-ops`: network rates, accounts and buckets, a trend page
  charged per request, Who-Is through the engine, and the decoder's tests
  (moved from the CLI).
- `crates/bacnet-cli`: the `[mcp]` table, authorization and its
  revocation, and the stdio hygiene test against the real binary.
