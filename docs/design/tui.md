# Design: a terminal UI for the `bacnet` CLI

Status: accepted (2026-10-01). Milestone M0's first issue, the scaffold and the
live devices screen (#961), and the B/IP profile/connection portion of #962
are implemented. The remaining #962 transports and the rest of epic #975 are planned. Spec references are clause numbers in ANSI/ASHRAE 135-2020,
paraphrased. Anything marked **planned** does not exist yet.

## 1. Summary

`bacnet` is a clap one-shot CLI plus a rustyline shell (`crates/bacnet-cli`).
This design adds a full-screen terminal UI, `bacnet tui`, built on ratatui 0.30
and crossterm 0.29. It is for people who commission, service and debug BACnet
networks, and for people building on rusty-bacnet. The one-shot commands stay
as the scriptable, JSON-producing and screen-reader-friendly interface.

The transports that matter most are BACnet/IP (including BBMD and
foreign-device operation), BACnet/SC (hub connection, failover, TLS
certificates) and MS/TP (RS-485). A keyboard-driven tool that covers all three
on Linux, macOS and Windows, with diagnostics built in, is useful in its own
right, and a TUI also runs over SSH on a box that is already on site, such as a
Raspberry Pi with a USB-RS485 adapter or a jump host inside an OT VLAN.

## 2. Goals and non-goals

**Goals**

- One `bacnet` binary on Linux, macOS and Windows that opens a TUI for
  discovery, browsing, guarded writes, live watching and transport diagnostics.
- Diagnostics per transport: BBMD and foreign-device health for B/IP, a
  connection doctor and certificate lint for SC, and a passive monitor with
  token-ring analysis for MS/TP.
- Read-only by default. Changes go through an explicit, time-boxed write
  session with previews, a revert ledger and an audit log.
- The one-shot CLI contract does not change: subcommands, aliases, flags,
  automatic JSON when piped, JSON and NDJSON shapes, errors on stderr with an
  empty stdout, and the SC error phases.
- The TUI uses public stack APIs only. Where the stack lacks something, the gap
  is filed and fixed in the stack rather than worked around in the CLI.

**Non-goals**

- A GUI, a web UI, or a replacement for Wireshark's decoder.
- Hosting a production server, router or SC hub from the TUI. A local lab hub
  is a later item.
- Generating credentials (a lab PKI).
- Scripting or macros, BACnet Ethernet, BACnet/IPv6-specific screens, COBS
  transmit, backup and restore, schedule and event-enrollment editing.
- Python parity: the wheel has no CLI entry point.

## 3. Users and workflows

Personas: commissioning technicians, service technicians, integrators, network
and IT/OT engineers, operators, and rusty-bacnet developers.

| ID | Workflow | Transports | Priority | Issue |
|---|---|---|---|---|
| W1 | What is on this network? Who-Is ranges, live I-Am table, duplicate instances; Who-Has; routers and networks | all | P0 | #961 (done), #960 (done) |
| W2 | Why can't I see device X? Interface and port, BBMD/FD, router path, SC handshake phase, MS/TP token | all | P0 | #962, #966, #967, #970 |
| W3 | Point checkout: browse, read values and status flags, override at a priority, release | all | P0 | #963, #964 |
| W4 | Watch live values (COV with renewal, polling fallback) | all | P0 | #965 |
| W5 | BBMD and foreign-device health: BDT consistency, FDT, TTL, NAT view | B/IP | P0 | #966 |
| W6 | SC onboarding: connect, verify certificates, diagnose TLS failures, primary and failover state | SC | P0 | #967, #968 |
| W7 | MS/TP trunk health: stations, token rotation, Max_Master gaps, errors; join safely | MS/TP | P0 | #969, #970 |
| W8 | Capture and decode traffic, filter, export pcap | all | P0 | #971 |
| W9 | Alarm summary and acknowledgement | all | P1 | follow-up |
| W10 | Trend log and schedule review | all | P1 | follow-up |
| W11 | Device admin: time sync, DCC, reinitialize, file read | all | P1 | follow-up |
| W12 | Lab: local SC hub, simulated devices | all | P1 | follow-up |
| W13 | Backup and restore, file write, create and delete object, schedule editing | all | P2 | later |

Alarm viewing is P1 because the client still returns GetEventInformation as raw
bytes even though `GetEventInformationAck::decode` exists; wiring the decoder in
is a small follow-up.

## 4. UX

### 4.1 Layout

The target layout once the later screens exist:

```
+ bacnet | site: plant-a | BIP 10.0.1.5:47808 | FD -> 10.0.0.1 (212 s) | READ-ONLY | COV 3 | drop 0 +
| 1 Devices  2 Browse  3 Watch  4 Network  5 SC  6 MS/TP  7 Capture                              |
+----------------------+------------------------------+--------------------------------------------+
| Devices (42)         | Objects: Device 1234 (318)   | AO:1 "Supply Fan Speed"                    |
| > 1234 10.0.1.20     |   analog-input (40)          |   present-value   42.0 %                   |
|   1301 2:0x05        | > analog-output (12)         |   status-flags    [OVR]                    |
+----------------------+------------------------------+--------------------------------------------+
| log: 12:01:03 INFO COV renewed AO:1 on 1234 (lifetime 300 s)                                    |
+ ? help  / filter  : command  W arm writes  u ledger  q quit -----------------------------------+
```

What M0 draws (80x24):

```
 bacnet | BIP 10.0.0.1:47808 | up | READ-ONLY | drop 0                   ? help
┌ Devices (5) | sort: instance ↑ ──────────────────────────────────────────────┐
│  Instance* Address                         Net    Vendor APDU  Seg      Seen │
│> 1001      10.0.1.21:47808                 local  260    1476  none     2s   │
│  2001      05 via 10.0.1.1:47808           2      24     480   both     7s   │
└ Who-Is local 1000-2999: done, 3 replies ─────────────────────────────────────┘
 d Who-Is  / filter  s sort  S reverse  L log  ? help  q quit
```

- The status bar shows the transport and local address, link state, the safety
  level, and a dropped-events counter. Later milestones add the site, the
  foreign-device countdown or SC hub state, and the remote footprint (active
  COV subscriptions and FD registrations).
- Screens appear only where they apply: Network for B/IP, SC for SC, MS/TP for
  a serial port. Devices, Browse, Watch and Capture are always present.
- 80x24 is the functional minimum and 120x40 the full layout. From 120 columns
  the device table uses its wide columns; below 120 the planned Browse panes
  stack and the focused pane fills the width. Below 80x24 the TUI shows a
  "terminal too small" notice and nothing else.

### 4.2 Screens

| Screen | Contents |
|---|---|
| Devices (M0) | Who-Is form (local, global, directed, remote network; instance range 0 to 4194303); live table (instance, address, network, vendor, max APDU, segmentation, last seen); sort and filter; duplicate-instance banner. **Planned:** Who-Has search and a Networks tab |
| Browse (#963) | Device card (vendor, model, firmware, protocol revision, services and objects supported, Max_APDU, segmentation, Database_Revision); Object_List grouped by type, with a filter; property pane with `[ALM]` `[FLT]` `[OVR]` `[OOS]` badges; priority-array pane with the slot in control marked. RPM with ALL, ReadProperty per property when RPM is refused, Object_List element by element when it can't come whole; one request outstanding per device and a `--max-rate` cap across devices |
| Watch (#965) | Watched points with value, units, flags, COV or polled, last-update age and a bar sparkline. Managed COV with a finite lifetime renewed before it ends (SubscribeCOVProperty off Present_Value), cancelled on remove and on exit; polling by batched RPM when COV is refused, at a per-point interval within the shared `--max-rate`; optional CSV log in the profile workspace |
| Network, B/IP (#966) | Foreign-device panel (BBMD, TTL, last BVLC-Result, countdown to the next registration, refusal banner); BDT with each mask explained, FDT with seconds left and our own entry marked ("what the BBMD sees me as"); BDT consistency matrix over every BBMD the tables lead to. `:bbmd ADDRESS` picks another BBMD. Write-BDT and Delete-FDT-Entry are later items |
| SC (#967) | Hub connector status (state, primary or failover hub, hub VMAC, reconnect attempt and backoff, heartbeat age and round trip); identity (local VMAC, device UUID, hub VMAC, duplicate-VMAC NAKs); the hub's certificate chain; the connection doctor's phase timeline (URL, DNS, TCP, TLS, WebSocket upgrade, Connect) with a hint from the typed failure; an event timeline. The local certificate inventory and pre-dial lint (#968) are a third view on the same screen |
| MS/TP (#969, #970) | Participant panel (`--mstp`), or the passive monitor (`tui --mstp-monitor PORT`): a station grid, the token ring with rotation and hold times, Max_Master findings, error counters, and the safe-join wizard (`J`) that hands the port to a participant through the T2 join dialog |
| Capture (planned) | Frame list, decode tree, hex view, filters, pcap export |
| Overlays | Help (`?`), log pane (`L`), interface/profile pickers and the B/IP editor/preview; arm dialog (`W`), write and relinquish forms (`w`, `n`), the preview and its confirmation, the session ledger (`u`) and the exit guard (#964) |

### 4.3 Keybindings

M0 keys:

| Key | Context | Action |
|---|---|---|
| `?` | global | show or hide help |
| `q` | global (not in text fields) | quit |
| `Ctrl-C` | global | close the open dialog or cancel the running Who-Is; a second press within 2 s quits |
| `L` | global | show or hide the log pane |
| `d` | Devices | open the Who-Is form |
| `/` | Devices | filter rows (substring of instance, address, network or vendor); Enter keeps it, Esc clears it |
| `s` / `S` | Devices | next sort column / reverse the order |
| `Enter` | Devices / Browse | browse the selected device / read the selected object |
| `Tab`, `Shift-Tab` | Browse | next or previous pane (below 120 columns only the focused pane is drawn) |
| `/`, `r`, `Esc` | Browse | filter objects; read again; back to the devices |
| arrows, `j`/`k`, `PgUp`/`PgDn`, `Home`/`End`, `g`/`G` | Devices | move |
| `Tab`, `Shift-Tab`, Up, Down | Who-Is form | next or previous field |
| Left, Right, Space | Who-Is form | change the scope |
| `Enter` | Who-Is form | send; a global or unbounded request needs a second Enter |
| `Esc` | dialogs | discard/back in the profile editor; close other dialogs |

Since M0: `a`, `x`, `3`/`1` and `:` for the watch list (#965); `W` to arm or
disarm writes, `w`/`n` to write or relinquish on an open object, and `u` for
the ledger (#964); `4` for the Network screen, with `r`, `m`, Tab and
`:bbmd` there (#966); `5` for the SC screen, with Tab between the hub's certificates, the doctor (#967) and this node's credentials (#968). Planned: `1` to `7` and `Tab` to switch screens, and
`Space`, `e`, `c` and `J` on later screens.

Only key presses count; Windows also reports releases. Mouse capture will be
opt-in (`--mouse`) because it breaks native text selection.

### 4.4 Accessibility

Full-screen redraws are close to unusable with screen readers, and ratatui has
no accessibility layer. Mitigations:

- Every TUI action has a one-shot or JSON equivalent. That is the supported
  accessible path.
- State is never shown by colour alone. Badges and markers are text
  (`READ-ONLY`, `DUPLICATE`, `>` for the selected row, `*` on the sorted
  column; later `[ALM]`, `[FLT]`, `[OVR]`, `[OOS]`). A snapshot test checks
  that the coloured and `NO_COLOR` frames carry the same text.
- `NO_COLOR` is respected: styles fall back to bold, dim and reverse video.
- `--fps` lowers the frame rate.

## 5. Safety model

The project README asks for authorization before use on a network. The
one-shot CLI and the shell guard writes with `--allow-write`, and DCC and
reinitialize also with `--yes` or a typed confirmation (#972). Both front ends
and the TUI take a command's class (read, write or device management) from one
table, `core/class.rs`; `Level::permits` maps the TUI's levels onto it.

**Session levels** (always shown in the status bar):

1. **Passive** (`--passive`, #964): never transmits. No client is built at
   all, so not even an automatic answer to an incoming request goes out; as a
   second line, a session loop running with the passive option refuses every
   command that would transmit. Today that leaves the ledger view and the
   MS/TP monitor (`tui --mstp-monitor`, #970), which opens its port
   listen-only and runs at this level with or without `--passive`; capture
   and offline files come later. A monitor's join wizard leaves this level
   for read-only once its T2 join is confirmed; with `--passive` it never
   joins.
2. **Read-only** (the default): discovery, reads, ReadRange, BDT and FDT reads.
   FD registrations create remote state with a finite TTL. Disconnect stops
   local renewal and releases the client; the remote FDT entry expires under
   its TTL plus grace rather than being deleted immediately (J.5.2.2–J.5.2.3).
   COV subscriptions are cancelled on remove and exit (#965). The status bar's
   footprint counts both (`COV n FD m`).
3. **Write session** (#964): armed with `W`. A modal shows the site and
   network, asks for an optional reason or ticket, and sets a duration (default
   15 min, at most 480). The bar turns red with a countdown and the session
   disarms when it expires; the worker keeps its own deadline too. A profile can
   set `writes = "never"`. Arming needs a profile workspace for the ledger and
   the audit log, so a flags-only session stays read-only.
4. **Dangerous operations** (#964): off unless the profile sets
   `dangerous = true` and a write session is armed. Today the only one is
   commanding priority 6; DCC, reinitialize and BDT writes will join it.

**Confirmation tiers**

| Tier | Actions | UX |
|---|---|---|
| T0 | Reads, discovery, capture | None, except that a global or full-range Who-Is gets a one-line warning with the expected response volume and needs a second Enter (M0) |
| T1 | Writes at priorities 9 to 16, non-commandable property writes | Preview (old to new, slot occupancy, whether the effective value changes), then Enter |
| T2 | Priority 8 overrides, Out_Of_Service toggles, WPM batches, relinquishing a slot we did not write, alarm ack, time sync, MS/TP join | Preview plus `y`; recorded in the ledger with the pre-image. An MS/TP join (#969) has no pre-image: it is logged (`L`, `--log-file`), not yet written to the ledger or the audit log |
| T3 | Priorities 1, 2 and 5, DCC, reinitialize, Write-BDT, Delete-FDT-Entry, create and delete object, file write | Shows the blast radius; the user types the device instance (or BBMD address) to confirm. DCC defaults to a finite duration |
| Blocked | Priority 6, which the standard sets aside for minimum on and off times (Clause 19.2.3); the deprecated plain-disable DCC option, which current-revision servers ignore (Clause 16.1) | Expert flag only |

As built in #964 (`tui/safety/plan.rs`): priorities 3, 4 and 7, which Table 19-1
leaves to the site but which outrank the operator's 8, are T2 like 8. A
WritePropertyMultiple batch is at least T2 and takes its strongest member's
tier; one blocked member blocks it. The "expert flag" is the profile's
`dangerous = true`, which turns priority 6 into a T3 write. Relinquishing a
slot this session's ledger holds is T1 at priorities 9 to 16. A write without a
priority goes to 16 for a commandable property (Clause 19.2.1); a priority on a
non-commandable property is not sent. The app enforces the tiers, and the write
worker checks the level, the policy and priority 6 again before sending. The T3
flow (the device instance typed out) and `audit::Secret` for passwords are
there for DCC and reinitialize to reuse.

Before any commandable write, the TUI reads the priority array, Present_Value,
Relinquish_Default and Out_Of_Service. It says when a stronger slot is active
(so the write would not change the present value) and when the target slot
already holds another writer's value: writing a slot replaces whatever another
party put there, and that party is not told (Clause 19.2). SERVICE_REQUEST_DENIED
from a device's mutation policy is shown as `DENIED`, apart from other errors.

**Session ledger** (#964). Every change records its pre-image (the slot value,
or the property's old value, Out_Of_Service included; BDT entries come with
BDT writes). `ledger.json` in the workspace is saved atomically before each
request goes out and after its answer, so a kill mid-request still leaves the
entry (state `sending`, or `unknown` after a timeout); a write is not sent if
the ledger can't be saved. Repeated writes to one target keep the first
pre-image, and a write that restores it closes the entry. Revert-all, newest
first, relinquishes our slots (or gives back another commander's value we
overwrote) and rewrites old property values; it needs a connection but not an
armed session. Quitting with entries left opens a modal: revert all, keep and
log, or cancel. The next session on the same profile lists entries an earlier
run left, marking the kept ones. A write already sent when its session is
retired completes and is recorded; one still queued is recorded as not sent.

Hardening after review (#964):
- A batch names each (object, property, index) once, checked in the plan and
  again in the worker: a WritePropertyMultiple error names its first failed
  write without the priority (Clause 15.10.1.3.2), so a repeat would be
  ambiguous. Writes before the failed one are tracked as applied, the failed
  one is not, and later ones are recorded as not sent.
- A slot is ours only while it holds what the ledger wrote; otherwise it is
  another commander's (T2 to relinquish, with a warning). A typed `null` is a
  relinquish. A Priority_Array that is read but isn't 16 values refuses a
  Present_Value write; only a missing or failed read means not commandable.
- The worker reads the targets again just before sending and refuses a
  preview whose pre-image or slot holder changed (audited as a stale
  decline). The app ignores Enter and `y` for 300 ms after a preview opens
  and declines a preview left for 120 s.
- The ledger update goes to the UI right after it is saved, and the exit
  guard also waits on an unanswered Prepare, Execute or Revert. Keep works
  without a session: the passive session or the coordinator flags the entries
  and records it.
- While a session's write worker runs it holds `ledger.lock` in the workspace
  (`std::fs::File::try_lock`, so `flock` or `LockFileEx`); another session on
  the profile can't arm and names the holder's process id where it can.
- A `sending` audit record precedes each request; on the next launch an
  earlier run's `sending` entries become `unknown` and are audited so.
  Revert goes by latest write and looks for an undiscovered device first.
- Raising the write policy (or a profile with no recorded authorized policy,
  read as authorized read-only) clears `authorized_at_unix`, so the
  attestation is asked for again; `authorized_safety` records what it
  covered.
- Audit records are written on the blocking pool; a session past its
  deadline but not yet disarmed is audited as `write-session (expired)`.

**Audit log** (#964). `audit.jsonl` in the profile workspace is append-only.
Each record holds UTC time, OS user, host, profile, transport and local
address, target, service, decoded parameters, priority, pre-image, outcome
(ack, error class and code, reject, abort, timeout) and session level.
Passwords are redacted (an `audit::Secret` serialises as `[redacted]`, and any
field named like a password is redacted on the way out), file payloads will be
reduced to size plus SHA-256 when file writes arrive, and key material never
appears. Arming, disarming, refusals, declined confirmations, reverts and kept
entries are recorded too. Each line is flushed and synced before the next; the
host name comes from `HOSTNAME`/`COMPUTERNAME` or the `hostname` command,
looked up once. This is a client-side record, separate from BACnet audit
reporting (Clause 19.6), and it complements the server-side
[mutation policy](../mutation-policy.md); SERVICE_REQUEST_DENIED is shown
distinctly.

**Other rules**

- Passwords for DCC and reinitialize are entered in masked fields and never
  stored, in history or anywhere else.
- Polling uses conservative defaults: a minimum interval per point, one
  outstanding confirmed request per device while browsing, and a global
  request-rate cap.
- An active MS/TP join is T2: joining a trunk means taking part in token
  passing (Clause 9.5), so a duplicate or out-of-range MAC affects every device
  on it. As built in #969, every join, a flags-only session's later ones
  included, shows the dialog and needs `y`; one-shot commands take `--mstp`
  as the request.
- The first use of a profile records an authorization attestation with a
  timestamp.

## 6. Architecture

### 6.1 Entry points and the one-shot contract

- `bacnet tui` opens the TUI. With no TTY on stdin or stdout, or with
  `TERM=dumb`, it exits non-zero with a hint on stderr and writes nothing to
  stdout, so `bacnet tui > out.txt` leaves `out.txt` empty.
- A bare `bacnet` and `bacnet shell` still open the rustyline shell. Switching a
  bare `bacnet` to the TUI, with `--no-tui` and `BACNET_NO_TUI` as escape
  hatches, is #974.
- The global transport flags (`-i`, `-p`, `-b`, `-t`, `--ipv6`, `--sc*`) feed
  both modes. B/IP profiles (§6.6) provide settings that explicit flags
  override, including values equal to the CLI defaults. Bare `tui` opens
  profile management; explicit network flags without `--profile`, or
  `--no-profile`, retain the nonpersistent connection path.
- The one-shot contract that must not change: subcommand names and aliases;
  global flags on either side of the subcommand; automatic JSON when not a TTY;
  JSON and NDJSON shapes with display-string values; errors on stderr with an
  empty stdout and a non-zero exit; stable `--sc-*` error phases; feature-off
  rebuild advice. Consumers include `scripts/release/cli_smoke.sh`, the README,
  the website guides and the integration tests in `crates/bacnet-cli/tests/`.
- The shell's hand-written argument parsers have drifted from clap. The planned
  command bar reuses shared parsing (`src/core/`) instead of adding a third
  parser.

### 6.2 Crate and module layout

Everything stays in `crates/bacnet-cli`, behind the `tui` cargo feature
(opt-in until the TUI ships in 0.14.0, #975), and outside the workspace's `default-members` as before. A separate
crate is not justified until a second consumer exists. Building without
the feature drops ratatui and crossterm; `bacnet tui` then prints rebuild
advice and exits 1.

```
crates/bacnet-cli/src/
  main.rs            dispatch: one-shot | shell | tui (the hand-built runtime from #950)
  args.rs            + Tui flags and explicit-value provenance
  core/              code shared by one-shot, shell and TUI: interface listing,
                     Who-Is range parsing (more as later screens need it)
  commands/          one-shot handlers (contract unchanged)
  tui/
    mod.rs           entry: TTY guard, tracing, interface choice, run and teardown
    terminal.rs      raw mode and alternate screen, panic hook, signals
    app/             App, Action, update() -> Vec<Command>; device table, Who-Is form, picker
    keymap.rs        keys to actions per context
    event_loop.rs    select! over input, frames, worker events and signals
    message.rs       Command and WorkerEvent: the UI <-> worker protocol
    log_layer.rs     tracing layer -> ring buffer
    profile/         versioned B/IP TOML store, overrides and pre-connect persistence
    worker/          coordinator + generic session; owns and retires the BACnetClient
    view/            pure rendering: devices table, chrome, dialogs
    tests/           app-core, frames (insta), in-process lab, load, event loop
```

`safety/` holds the write session model (levels, tiers and planning, ledger,
audit writer); `worker/write.rs` and `worker/passive.rs` its worker side, and
`app/safety.rs` and `view/safety.rs` its dialogs (#964). Planned: one worker
and view module per later screen. The implemented profile
store is in `profile/`; SC and MS/TP persistence are still planned.

The 700-line file cap (`scripts/ci/check-file-size.sh`) and `missing_docs =
deny` apply. The TUI writes through the crossterm backend, not `print!`, so the
workspace `print_stdout` lint is not involved.

### 6.3 Async model

```
crossterm EventStream ----+
frame interval (20 fps) --+--> UI loop on the block_on thread
worker events (bounded) --+      update(&mut App, Action) -> Vec<Command>
signals ------------------+      if dirty: terminal.draw(view)
                                       |
                                       v
                          bounded command channel -> worker task (tokio::spawn)
                                                       uses BACnetClient<T>
```

- **UI loop.** It runs inside the boxed `cli_main` future that the hand-built
  runtime `block_on`s (#950), not in `tokio::spawn`, so a slow `terminal.draw`
  never stalls a worker. Each pass is a `select!` over the crossterm
  `EventStream` (key presses only), a frame interval (default 20 fps, set with
  `--fps`), the bounded worker-event channel, and the termination signals.
- **Redraws.** The state carries a dirty flag. A key press that changes it
  redraws straight away, so typing feels immediate; worker events only mark it
  dirty and are drawn at the next frame tick, so a burst costs one frame however
  many events it holds. Ages in the table tick once a second, so an idle screen
  is not redrawn at all. Worker events are applied up to 256 per wake-up before
  input and frames get a turn.
- **Worker.** One task owns the client. It takes bounded `Command`s and emits
  `WorkerEvent`s; operations carry an id, and `update` drops results for an
  operation that is no longer current. The UI closes the command channel to
  stop it, and the worker stops the client.
- **High-rate streams.** The client's device broadcast is folded into the
  bounded worker-to-UI channel (1,024 events). When the UI falls behind, events
  are dropped and counted, and a lagging broadcast receiver adds its count; the
  status bar shows the total. After any loss the worker resynchronises the table
  from the client's own discovery table, at most once a second. Duplicate-
  instance notices are rare and important, so they wait for room instead of
  being dropped; the record behind the banner keeps at most four addresses per
  instance and counts the rest, and its text is rebuilt only when it changes.
  Lost devices leave the table with a line in the log pane. Later streams
  (COV, events, frames) get the same treatment.
- **Latest-value state** (SC link state, MS/TP statistics, FD countdown) will
  use `watch` channels.
- **MS/TP** will run with `MstpExecutionMode::DedicatedThread`, so terminal
  stalls cannot reach the MAC loop; Clause 9.5.3 timing works in tens of
  milliseconds.
- **Stack size** (#953). Windows gives the main thread 1 MiB, and debug futures
  are large. The TUI future is boxed, `App` is boxed, the worker and each
  client build are boxed futures on runtime threads, and the draw path is
  small. The CLI tests run under the native jobs' 1 MiB stack guard.
- **Cancellation.** In raw mode Ctrl-C arrives as a key, not SIGINT, so
  cancellation is key-driven: Ctrl-C closes the open dialog or cancels the
  running Who-Is, and a second press within two seconds quits.
- **Terminal ownership.** In TUI mode tracing goes to an in-app ring buffer
  (1,000 lines, shown in the `L` pane) and optionally to `--log-file`. Nothing
  writes to stdout or stderr while raw mode is on; that is why the BIP
  interface picker is a dialog rather than the shell's stderr prompt.
- **Teardown.** Setup mirrors `ratatui::init` and teardown calls
  `ratatui::try_restore`. The panic hook is our own so that it shares one
  idempotent restore with normal exit and signal exit and so that its order is
  testable: it restores the terminal first, then runs the previous hook, which
  prints the panic on a usable screen.
- **Panics off the UI thread.** The hook is process-wide, so a panic in the
  worker or in any client or transport task also restores the terminal, and
  tokio then catches it. The loop checks `terminal::is_active()` before every
  pass and every draw, so it never paints over the restored screen; it quits
  with an internal error and exit status 1. The worker's channel closing while
  the UI still runs is treated the same way.
- **Signals and exit status.** SIGTERM, SIGHUP and an external SIGINT (Unix),
  and console close and Ctrl-Break (Windows), end the loop and restore the
  terminal. On Unix the TUI then exits with 128 plus the signal number, as
  shells report it (143, 129, 130); on Windows it exits 1. The handlers stay
  installed while the worker shuts down, so a Ctrl-C during that wait (SIGINT
  again, now that the terminal is cooked) is recorded for the eventual exit
  status. The coordinator retains every client build and shutdown future and
  joins its worker before exit; after three seconds it logs that resource
  release is still pending. It never drops a timed-out join handle. Windows ends a process soon after console close, so that restore
  is best effort. A terminal that passes the TTY check but refuses raw mode
  (MSYS and mintty) gets the same hint as the TTY check and exit status 1.

### 6.4 State model

TEA-style: one `App` struct, one `Action` enum, and an `update` function with no
I/O that returns `Vec<Command>`. Time reaches `update` only through
`Action::Tick`. Views are pure functions of `&App` (the table keeps its scroll
offset in a `Cell` between frames), and the keymap turns keys into `Action`s.
Components do not get their own channels or locks, which keeps every state
transition testable without a terminal.

```
App { transport, link, devices: DeviceTable, duplicates, overlay, op, flash,
      now, dropped, log: LogRing, quit, exit_error, dirty, .. }
Action = Tick { now, dropped, log_generation } | Resize | Quit | CtrlC | Escape
       | ToggleHelp | ToggleLog | OpenWhoIs | Move | CycleSort | ReverseSort
       | StartFilter | Filter | Form | Picker | Worker(WorkerEvent)
Command = Connect { interface, broadcast } | WhoIs { op, spec } | Cancel { op }
```

Tables keep sorting and filtering in the model and render only the visible
rows, which matters for devices with up to 10,000 objects; drawing the device
table at 120x40 with 4,096 rows must take under 5 ms in a release build.

### 6.5 How the TUI talks to the stack

The TUI uses only public `bacnet-client` and `bacnet-transport` APIs.

- **Generics.** The worker is generic over `T: TransportPort` and instantiated
  per transport, as the one-shot commands are. `App` and the views are not
  generic, so monomorphization stays in the worker layer.
- **APIs used by M0:** `who_is`, `who_is_directed`, `who_is_network` and
  `broadcast_unconfirmed` (for a local-only Who-Is), `device_events()`,
  `device_collision_events()` and `discovered_devices()`.
- **APIs later screens will use:** RP and RPM, WP and WPM (local and routed);
  `subscribe_cov`, `manage_cov_subscription` and `cov_notifications()`; the BBMD
  helpers on any `BACnetClient` whose transport is `AsBip` (`BipTransport`, or
  `AnyTransport` with a `Bip` variant); `ScConnectError` and
  `ScWebSocketErrorKind`; the MS/TP `diagnostics()` handle and
  `decode_frame_stream`.
- **Transport handles.** `client.transport()` (#956) borrows the transport of
  any built client. Workers take the owned handles once (the SC
  `connection_state_changes()` watch, the MS/TP `diagnostics()` handle, the
  BBMD state `Arc`) and poll the counter snapshots (SC `npdu_drop_counts()`,
  B/IP management, FDT and fanout counters) through the borrow. They never
  send through the transport or hold the BBMD state lock across an await.
- **Monitor and capture.** The MS/TP passive monitor has no client: its
  session owns the receive-only port and the `MstpMonitor` (#958, #970), and
  stops the monitor before it returns, so the coordinator opens a joining
  participant only after the port is closed. The capture screen (#971) can
  take its own lossy subscription to the monitor's frame stream. Live capture
  of our own traffic needs a link-level tap (#957).

### 6.6 Profiles and workspaces (#962)

B/IP and BACnet/SC profiles are implemented behind `tui`. The entry flow,
schema and keys are in the [CLI reference](../CLI.md#terminal-ui). MS/TP
profiles (#969) add a `transport` value and their own table. An SC profile's
failover hub goes to the client builder with the default reconnect policy,
which the builder needs for failover; the SC screen (#967) shows which hub is in use. The shell
history lives in the platform data directory.

- Each transport keeps its settings in its own table (`[bip]` or `[sc]`), and
  exactly the one `transport` names is present. B/IP files from before SC
  support have that shape already and load unchanged.
- SC profiles hold the hub URLs, the CA, certificate and key paths, the VMAC
  and the device UUID. The VMAC and UUID are generated once, when the profile
  is first saved or first connected, and persisted: Annex AB.1.5.3 has the
  device UUID stay fixed for the life of the device.
- File checks (readable CA, certificate and key; on Unix a warning for a
  group- or world-readable key; with `sc-tls`, the `sc-check` credential lint,
  #968) run at the coordinator boundary for the preview, and again before a
  connection attempt, which an error refuses before any client exists.
  Warnings hold the attempt until `y` on the preview accepts the ones shown;
  the connect request carries them and the coordinator compares them with its
  fresh checks. `build_sc_client` applies the lint's hard failures once more,
  for flags-only sessions and one-shots.

- Version 1 TOML files live at `profiles/NAME.toml` under
  `directories::ProjectDirs::from("", "", "bacnet").config_dir()`. Set
  `BACNET_CONFIG_DIR` to replace that root. Per-profile `workspaces/NAME/`
  directories use the same root and hold the session ledger, the audit log
  (#964) and watch-list CSV logs (#965); no device cache or capture yet.
- Names are bounded ASCII identifiers, checked before constructing paths.
  New files cannot overwrite an existing profile. Saves publish a complete,
  synced temporary file atomically and preserve the prior file on failure.
  Reload and stale-preview checks detect changed documents; this is not a
  cross-process editing lock or crash-durability guarantee for the directory.
- B/IP settings cover the local IPv4 address, UDP port (including 0),
  broadcast address, timeout, pacing, optional BBMD, TTL and renewal. The
  editor clears renewal when the BBMD is cleared, returning to Normal mode.
  Save errors retain the draft; Esc discards it and returns to a preview or
  picker where reload is reachable. Stale saves still preserve external edits.
  Deterministic invalid settings are rejected before binding. Whether the
  BBMD equals the actual local endpoint is checked by the transport after bind.
- `writes = "never" | "session"` (default `session`) and `dangerous`
  (default false; true needs `session`) are the write policy (#964). First use requires
  an explicit authorization attestation, saved with a Unix timestamp before
  construction of the client. A save/workspace failure prevents connection.
  Editing saved network settings clears the attestation. CLI overrides stay
  session-local, including during the authorization save.
- The connection dialog previews the resolved settings and requires
  disconnection before editing or selecting another profile. Rejected operation
  commands carry generation and operation identity separately from completion.
  A refused cancel corrects optimistic cancellation and offers immediate retry
  or disconnect; ordinary late progress cannot erase that failure. One retained
  coordinator owns attempts and workers; disconnect is acknowledged only
  after stop and join, and a replacement starts after retirement. Session
  generations reject late connection, discovery, snapshot and collision
  events. Bounded command rejection is visible rather than silently dropped.
- A local foreign-device start and accepted remote registration are distinct.
  The first registration outcome appears in the log pane; a refusal or timeout
  keeps the existing background renewal policy. Disconnect stops renewals,
  without promising immediate remote FDT deletion.


## 7. Crate choices

| Crate | Version | MSRV | Licence | Use |
|---|---|---|---|---|
| ratatui | 0.30.2 | 1.88 | MIT | core; default features minus the calendar widget (which pulls in `time`) and macros |
| crossterm | 0.29 | 1.63 | MIT | backend with `event-stream`, imported as `ratatui::crossterm` |
| futures-util | workspace 0.3 | | MIT/Apache-2.0 | `StreamExt` on `EventStream` |
| own tracing layer | | | | the log pane (a small `Layer`; avoids `log` and `env_filter` through tui-logger) |
| dev: insta | 1.48 | 1.66 | Apache-2.0 | snapshot tests |
| dev: tokio `test-util` | workspace | | MIT | paused-time tests |
| toml / directories | 1.1 / 6.0 | within workspace MSRV | MIT/Apache-2.0 | versioned B/IP profiles and platform config paths, behind `tui` |
| later, if needed | tui-tree-widget 0.24 (capture decode tree), tui-input 0.15 (command bar), x509-parser 0.18 behind `sc-tls` (certificate lint), tui-popup, tui-scrollview, nucleo-matcher (MPL-2.0, allowed by `deny.toml`), portable-pty and vt100 (PTY end-to-end tests) | | | |
| skipped | color-eyre, better-panic, human-panic, template config stacks | | | the panic hook above already restores the terminal |

- **MSRV.** The highest requirement is 1.88 (ratatui), against a workspace MSRV
  of 1.93. ratatui has raised its MSRV in a patch release before, so the
  workspace uses `resolver = "3"`, which prefers dependency versions compatible
  with the declared `rust-version` when the lock file is updated. The committed
  lock file, `--locked`, and the MSRV CI job remain the backstop.
- **Size.** Measured for #961 on Linux x86_64 release builds, the `tui` feature
  adds about 1.0 MB to the default `bacnet` (7.20 to 8.20 MB; 5.40 to 6.13 MB
  stripped) and 1.1 MB with `sc-tls`, as the release assets are built (13.72 to
  14.84 MB; 10.84 to 11.68 MB stripped). The workspace has no
  `[profile.release]` tuning (strip, LTO) yet; #973 measures it before
  deciding.
- **Duplicates.** The only new semver-incompatible duplicate in the built
  graph is `hashbrown` 0.16 (via `kasuari`). `Cargo.lock` also gains entries for
  ratatui's optional backends (termwiz, termina, with `thiserror` 1), which are
  never built.

## 8. Testing strategy

| Layer | What | Where it runs |
|---|---|---|
| a. App core | `update()` driven by synthetic keys and worker events; no terminal | all three native CI OSes |
| b. Frames | `TestBackend` frames at 80x24 and 120x40 as insta snapshots; a check that colour never carries information the text lacks | all OSes |
| c. In-process loopback | real `BACnetServer`s and a `BACnetClient` on an in-memory hub transport, driven through the worker and `update`: range and unbounded Who-Is, duplicate instances. Later: BBMD-mode BIP, a `BACnetRouter` between two networks, an mTLS `ScHub` | all OSes |
| d. Event loop | the `select!` loop on a `TestBackend` with scripted input and paused time: redraw-only-when-dirty, presses only, Ctrl-C, end of input | all OSes |
| e. Load | 1,000 device events per second against a stalled UI: the channel stays bounded and every event is delivered or counted; render time of a 4,096-row table | all OSes (the 5 ms budget is asserted in release builds) |
| f. Contract | the existing one-shot integration tests, unchanged; `bacnet tui > out.txt` fails with a hint and an empty file | all OSes |
| g. Docker topology | `examples/docker/docker-compose.yml` (bbmd-a/b, router, sc-hub, foreign-client) | manual and release checks |
| h. MS/TP | `LoopbackSerial::pair()`, PTY `SerialStream::pair()`, the scripted ring of #958 (`bacnet-transport`'s `test-support`, played into the monitor on paused time); real USB-RS485 adapters for hardware checks | Linux for PTY; loopback and the scripted ring everywhere |
| i. PTY end-to-end (planned) | start, quit, terminal restored, with portable-pty and vt100 | Unix first |

Rules: fixed terminal sizes; times as fixed offsets so ages render the same on
every run; addresses from the in-memory hub rather than ephemeral ports.

**Running the snapshot tests.** The tests live in
`crates/bacnet-cli/src/tui/tests/` and the accepted frames in its `snapshots/`
directory. CI and a plain `cargo nextest run -p bacnet-cli` compare against the
committed `.snap` files and fail on any difference. After an intended UI
change, regenerate and review them with
[cargo-insta](https://insta.rs/docs/cli/):

```bash
cargo insta test --test-runner nextest -p bacnet-cli   # writes .snap.new files
cargo insta review                                      # accept or reject each
```

The release-build render budget runs with:

```bash
cargo nextest run -p bacnet-cli --release -E 'test(drawing_a_full_device_table)' --no-capture
```

## 9. Platform support

| Capability | Linux | macOS | Windows |
|---|---|---|---|
| TUI (crossterm) | yes | yes (Terminal.app, iTerm2) | Windows Terminal; conhost on Windows 10 1703 and later. mintty and Git Bash are not supported terminals |
| B/IP | yes | yes | yes; wildcard local-address discovery is missing (#952) |
| BACnet/SC | yes | yes | yes |
| MS/TP participant (#969) | yes, in the release binary: USB auto-direction (kernel RS-485 and `serial-gpio` stay library-only) | experimental, source builds with `serial`: USB auto-direction, `/dev/cu.*` | experimental, source builds with `serial`: USB auto-direction, `COMn` |
| MS/TP passive monitor (#970) | yes, wherever the participant is | experimental | experimental (host timer resolution limits timing accuracy) |
| Own-traffic capture (planned) | yes | yes | yes |
| Wire capture (libpcap) | yes (root or CAP_NET_RAW) | builds | needs the Npcap SDK |

- The `serial` feature is not Linux-only: tokio-serial and serialport build on
  all three OSes. Only kernel RS-485, `serial-gpio` and `ethernet` are
  Linux-only.
- MS/TP hardware evidence exists only for Linux USB adapters
  ([MS/TP qualification](../mstp-qualification.md)). USB-serial latency (the
  FTDI latency timer) can eat into the Clause 9.5 timing budget.
- Port names: Linux needs the `dialout` group; on macOS use `/dev/cu.*`, not
  `tty.*`.

## 10. Stack gaps (separate issues the TUI depends on)

| Issue | Gap | Needed by |
|---|---|---|
| #956 | Done: `BACnetClient::transport()` and the `AsBip` BBMD helpers. Was: no access to transport handles from a built `BACnetClient`: the SC state watch, SC drop counts and BIP counters are unreachable after build, and the BBMD helpers exist only on `BACnetClient<BipTransport>`, so `AnyTransport` loses them | #967, #957, #959 |
| #957 | No link-level frame tap on any transport, so BVLC control traffic, SC control messages and MS/TP Token, PFM and Reply-Postponed frames are invisible | #971 |
| #958 | No MS/TP passive monitor, per-station statistics or master list; `MstpDiagnostics` not exposed from builders | #970, #971 |
| #959 | SC reconnect, failover and disconnect causes appear only as tracing lines; no active-hub indication, heartbeat statistics or peer certificate; no failover or reconnect options on `ScClientBuilder` | #967 |
| #960 | Done: `i_have_events`, `who_has_replies`, `network_reports`, `discover_routers`; `find`, `whois-router`, `:find` and the Networks tab. Was: I-Have is dropped; the client can't send Who-Is-Router-To-Network or What-Is-Network-Number or receive I-Am-Router-To-Network (Clause 6.4); the CLI `find` and `whois-router` are stubs | W1 (Who-Has, network map) |
| #938 | Foreign-device mode on `BipClientBuilder`; surfacing a refused registration (BVLC-Result NAK); a separate re-registration interval; `discover --bbmd` should send Distribute-Broadcast-To-Network (Annex J.5) | #966 |

Later gaps, not filed yet: routed variants for ReadRange, file access, DCC,
reinitialize, time sync, GetEventInformation, AcknowledgeAlarm and create and
delete object; GetEventInformation decoded in the client; client wrappers for
GetAlarmSummary, GetEnrollmentSummary, PrivateTransfer and TextMessage; client
TSM counters; an opt-in SC hub admin view of connected nodes.

## 11. Risks

| Risk | Mitigation |
|---|---|
| crossterm has not released since 0.29.0 and has breaking changes queued | Import through `ratatui::crossterm`; prefer widgets without a direct crossterm dependency |
| ratatui raises its MSRV in patch releases | Committed lock file, `--locked`, `resolver = "3"`, the MSRV CI job |
| Windows terminal quirks (duplicate key events, conhost resize, glyphs, mintty) | Press-only key filter; per-frame size; ASCII and box-drawing glyphs only; native Windows CI |
| TUIs are inaccessible to screen readers | One-shot parity for every action; text badges; `NO_COLOR`; low frame rate option |
| Main-thread stack overflow on Windows (#953) | Small draw path; boxed futures and state; worker on runtime threads; no server or hub hosting; 1 MiB stack guard in CI |
| Unbounded channels hide overload | Bounded channels; visible drop counter; per-wake drain budget; resync after loss |
| MS/TP timing on USB adapters, macOS and Windows | DedicatedThread; passive monitor by default; label macOS and Windows experimental until hardware evidence exists |
| An active MS/TP join or careless writes disturb a live site | Read-only default, write sessions, tiers, ledger, exit guard, safe-join wizard |
| Breaking the one-shot contract | The TUI is additive; contract tests stay unchanged; the shell is retired only by decision (#974) |
| Scope creep into a second product | Milestones ship independently; P1 and P2 items need their own issues |
| Audit logs leak secrets | Audit records redact passwords and never contain key material; SC key-log export is out of scope |
| Compile time and binary size growth | Non-generic views; the `tui` feature can be turned off; release-profile tuning measured in #973 |

## 12. Milestones

- **M0 foundation:** #961 (scaffold and devices screen, done), #962 (profiles
  and connection setup).
- **M1 BACnet/IP:** #963 (browser), #964 (safety and writes), #965 (watch
  list), #966 (BBMD and foreign device).
- **M2 BACnet/SC:** #967 (status and doctor), #968 (certificate lint).
- **M3 MS/TP:** #969 (MS/TP transport in the CLI), #970 (monitor and safe join).
- **M4 live capture:** #971.
- **Stack gaps:** #956 to #960, plus #938.

## 13. Decisions (maintainer, 2026-10-01)

- **Entry point:** ship `bacnet tui` first; a bare `bacnet` switches to the TUI
  after M1 and the shell is retired later (#974).
- **Packaging:** one binary with a default-on `tui` feature; the code lives in
  `tui/` inside bacnet-cli with a shared `core/`.
- **Terminal size:** 80x24 functional, 120x40 full layout, a notice below the
  minimum.
- **MSRV and resolution:** `resolver = "3"` with MSRV 1.93; the MSRV CI job is
  the backstop.
- **Profiles:** TOML in the platform config directory with an environment
  override; flags-only runs stay supported.
- **Audit log:** on by default for mutations and safety events.
- **Certificates:** x509-parser behind `sc-tls`; where our hostname and IP check
  is stricter than the Annex AB.7.4 default, explain it rather than relax it.
- **MS/TP release status:** in the Linux CLI release for 0.12.0; experimental in
  macOS and Windows CI until there is hardware evidence for at least one USB
  adapter per OS.
- **One-shot write guard:** a separate issue (#972), which changed the one-shot
  contract on purpose for 0.14.0: mutating commands need `--allow-write`.
- **Lab PKI generator:** out of scope for v1.
- **Release profile size tuning:** measured separately (#973) before deciding.
- **Timing:** the TUI targets 0.14.0, so it holds up neither 0.12.0 nor
  0.13.0.

## 14. References

- Ratatui 0.30 highlights <https://ratatui.rs/highlights/v030/>, backends
  <https://ratatui.rs/concepts/backends/comparison/>, the Elm architecture
  <https://ratatui.rs/concepts/application-patterns/the-elm-architecture/>,
  snapshot testing <https://ratatui.rs/recipes/testing/snapshots/>, panic hooks
  <https://ratatui.rs/recipes/apps/panic-hooks/>
- crossterm <https://github.com/crossterm-rs/crossterm>
- insta <https://insta.rs/>
- Spec clauses (paraphrased above): 6.4 network-layer messages; 9.5 MS/TP; 16.1
  DCC; 16.9 Who-Has; 16.10 Who-Is; 19.2 command prioritization; 19.6 audit
  reporting; Annex J (J.5) B/IP and BBMD; Annex AB (AB.1.5, AB.7.4) BACnet/SC
- Repo: `crates/bacnet-cli/`, `crates/bacnet-client/src/client/`,
  `crates/bacnet-transport/src/`, [CLI reference](../CLI.md),
  [MS/TP qualification](../mstp-qualification.md),
  [mutation policy](../mutation-policy.md), [CI](../ci.md)
