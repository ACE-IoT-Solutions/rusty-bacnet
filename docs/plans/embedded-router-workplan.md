# Embedded BACnet/IP↔MS/TP Router Workplan

Status: active; R0 complete, R1 next

Last updated: 2026-10-11

Scope: portable library components only; firmware integration and hardware
validation remain downstream responsibilities.

## Architecture decision

The hosted `bacnet-network` router remains the runtime implementation for
Tokio transports. It cannot be used directly on ESP-IDF because it owns Tokio
channels/tasks, `std` clocks and hosted `bacnet-transport` ports. Embedded
routing is therefore implemented in the independent `bacnet-router-core`
crate, following the synchronous, caller-clock pattern already used by
`bacnet-mstp-core`.

The core must not own sockets, UARTs, executors or unbounded queues. A platform
adapter provides ingress metadata and implements synchronous egress admission.
`AdmissionOutcome::AcceptedOwned` means the actual data-link queue copied or
took ownership of the NPDU before returning. Router scheduling alone is not
success.

## Fixed invariants

- Validate the complete NPDU length after routing-header rewriting against the
  selected port's `max_npdu_len`; never use APDU or ingress length as a proxy.
- Keep configured direct-route capacity independent of learned-route capacity.
  Discovery traffic cannot evict a configured network.
- Preserve immediate UDP peer, actual local destination/group provenance and
  logical Forwarded-NPDU origin as distinct values.
- Keep network-control capacity distinct from ordinary data capacity in every
  concrete adapter.
- Reject malformed control payloads before any routing-table mutation.
- Never truncate or fragment an NPDU in the router.
- Standard MS/TP remains capped at a 501-byte NPDU until the Extended MS/TP
  work item below is complete end to end.

## Implementation phases

Phases are ordered by dependency. Each phase should land as one focused PR, or
as the smaller PR slices listed under that phase. A phase is complete only when
its exit criteria pass; compiling a type or codec alone is not completion.

| Phase | State | Depends on | Capability unlocked |
|---|---|---|---|
| R0 Portable boundary/direct forwarding | Complete | — | Bounded direct unicast foundation |
| R1 Complete unicast routing | Next | R0 | Direct and learned unicast with explicit refusal |
| R2 Router discovery/table control | Planned | R1 | Dynamic route discovery and management |
| R3 Routed broadcasts/loop safety | Planned | R1, R2 | Bounded remote/global broadcast forwarding |
| R4 Adapter composition | Planned | R1–R3 | Library-complete standard-frame B/IP↔MS/TP router |
| R5 Diagnostics/control | Planned | R2–R4 | Operationally supportable embedded router |
| X0–X5 Extended MS/TP | Planned, claim blocker | R0; converge with R4 | Extended DER/DNER transport to 1497-byte NPDU |
| R6 Release evidence | Planned | R0–R5; X0–X5 for broad claim | Evidence-backed release/claim decision |

### R0 — Portable boundary and direct forwarding (implemented)

- [x] Add allocation-free `bacnet-router-core`.
- [x] Add strict borrowed BVLL decode and caller-buffered encode.
- [x] Require actual UDP destination metadata and preserve immediate/logical
  source identities.
- [x] Add independently bounded direct and learned route storage.
- [x] Add direct-network NPDU rewrite with priority preservation.
- [x] Add per-port final-NPDU MTU validation.
- [x] Add synchronous owned-admission results and diagnostic counters.
- [x] Prove the B/IP→standard-MS/TP 501-byte accept / 502-byte reject boundary.

This phase is foundation work, not a claim that the embedded router is
complete.

### R1 — Complete unicast routing

Goal: make the portable core authoritative for all unicast forwarding outcomes,
without yet learning routes from network-control messages.

Deliverables:

- [ ] Add one validated borrowed NPDU view used by direct and learned routing.
- [ ] Forward through learned next hops, retaining DNET/DADR and decrementing
  hop count exactly once.
- [ ] Preserve or insert SNET/SADR correctly for both direct and learned paths.
- [ ] Preserve standard and proprietary network-message type/vendor fields
  byte-for-byte.
- [ ] Reject zero input hop count before admission; allow the defined
  intermediate `1 → 0` transition without underflow.
- [ ] Reject unavailable/busy routes deterministically and retain the route's
  owning next-hop identity.
- [ ] Model Reject-Message-To-Network as a core output, including message-too-
  long for a final rewritten NPDU that exceeds egress capability.
- [ ] Retain enough ingress context to route a refusal toward the original
  source rather than only the immediate peer.
- [ ] Keep `Full`, `Oversize`, `PortDown`, malformed input and no-route as
  distinct observable results.

Suggested PR split:

1. R1a — borrowed NPDU view plus direct/learned rewrite builder;
2. R1b — learned-route selection, hop behavior and next-hop admission; and
3. R1c — rejection actions and refusal-path diagnostics.

Exit criteria:

- [ ] Exact-wire tests cover direct and learned application NPDUs, standard
  network messages and proprietary network messages.
- [ ] Boundary tests cover hop counts 0, 1 and 255.
- [ ] Final-size tests cover packets that grow and shrink during rewrite.
- [ ] Egress is never called for malformed, exhausted, unavailable, no-route
  or pre-admission oversize traffic.
- [ ] Queue-full/port-down/adapter-oversize outcomes retain source context and
  increment exactly one counter.

### R2 — Router discovery and table control

Goal: discover and manage routes without allowing malformed or unauthoritative
control traffic to partially mutate state.

Deliverables:

- [ ] Add borrowed parsers and caller-buffered encoders for the supported
  Clause 6 network-control messages.
- [ ] Implement strict Who-Is-Router-To-Network query handling, including the
  all-networks form and forwarding of unknown specific-network queries.
- [ ] Implement I-Am-Router-To-Network learning, announcements and controlled
  rebroadcast.
- [ ] Parse the entire Initialize-Routing-Table or ACK payload before applying
  any entry; reject truncation, trailing bytes and invalid/reserved networks.
- [ ] Define and enforce Initialize-Routing-Table port-ID semantics for this
  core rather than copying hosted port indices implicitly.
- [ ] Apply Router-Busy/Router-Available only when the ingress port and next-hop
  MAC own the learned route; never mark configured direct routes busy.
- [ ] Add caller-time route aging, busy expiry and explicit learned-route clear.
- [ ] Add detached route snapshots that expose no live mutable storage.
- [ ] Define behavior for unsupported standard, security and proprietary
  network messages without broadening protocol claims.

Suggested PR split:

1. R2a — strict control-message codec and truncation matrix;
2. R2b — Who-Is/I-Am discovery and route learning;
3. R2c — Initialize-Routing-Table transaction; and
4. R2d — Busy/Available ownership, aging and snapshots.

Exit criteria:

- [ ] Every truncation offset and malformed entry leaves the table unchanged.
- [ ] A full learned table can refresh an existing entry but cannot evict or
  overwrite a direct route.
- [ ] Control response admission failures are visible and never reported as
  successful announcements.
- [ ] Deterministic caller-time tests cover exact busy and aging boundaries.
- [ ] Hosted and portable control codecs share byte fixtures where their scope
  overlaps.

### R3 — Routed broadcasts and loop safety

Goal: support routed remote/global broadcasts without unbounded amplification
or silently suppressing legitimate repeated requests.

Deliverables:

- [ ] Add explicit policy for disabled, remote-network and global broadcast
  forwarding; default fail-closed until configured.
- [ ] Exclude the ingress port and decrement routed hop count exactly once per
  forwarded copy.
- [ ] Add a fixed-capacity, caller-time echo cache keyed by canonical origin,
  destination semantics and packet identity, with hop count normalized out.
- [ ] Scope suppression to routed echoes; do not use an indefinite blanket
  `(source, payload)` duplicate rule.
- [ ] Define deterministic cache-full behavior and expiry/replacement policy.
- [ ] Reserve separate control admission so saturated data queues cannot block
  discovery, rejection or congestion signaling.
- [ ] Return per-egress fanout results rather than collapsing partial success
  into a single forwarded outcome.

Suggested PR split:

1. R3a — broadcast policy and deterministic fanout;
2. R3b — bounded echo cache and canonical key; and
3. R3c — reserved control admission and partial-fanout diagnostics.

Exit criteria:

- [ ] Tests cover remote and global broadcast, ingress exclusion, mixed
  accepted/full/down ports and exact hop behavior.
- [ ] Alternate-ingress loops are suppressed within the configured window.
- [ ] An identical legitimate transmission is admitted after expiry.
- [ ] Cache saturation is bounded, deterministic and observable.
- [ ] I-Am-Router and other hopless control rebroadcasts cannot circulate
  indefinitely between router ports.

### R4 — Concrete adapters and composition

Goal: provide the complete library seam the firmware team can drive, while
leaving ESP-IDF task/socket/UART ownership downstream.

Deliverables:

- [ ] Stabilize the ESP-IDF-facing UDP contract for datagram source, actual
  destination, group classification, truncation and monotonic timestamp.
- [ ] Keep socket calls outside `bacnet-router-core`; provide an adapter that
  converts those facts into `BipIngress` and executes caller-buffered BVLL send
  actions.
- [ ] Define ordinary-node scope explicitly. BBMD/FDT, foreign-device, NAT and
  B/IP-M behavior remain separate unless deliberately added.
- [ ] Bridge router admission directly to `bacnet-mstp-core` queues so
  `AcceptedOwned` means that queue owns the bytes.
- [ ] Preserve NPDU priority rather than forcing all MS/TP submissions to
  Normal.
- [ ] Map MS/TP `Full`, `Oversize` and `Expired` distinctly without adding a
  second hidden backlog.
- [ ] Add reverse MS/TP→B/IP address conversion and Original-Unicast/Broadcast
  selection.
- [ ] Add composed in-memory B/IP→router→MS/TP and MS/TP→router→B/IP tests.
- [ ] Add local bare-target dependency and file-size checks for the new crate;
  changing CI remains a separate explicitly authorized task.

Suggested PR split:

1. R4a — stable UDP/BVLL adapter contract;
2. R4b — direct MS/TP core admission adapter; and
3. R4c — bidirectional composed tests and bare-target validation scripts.

Exit criteria:

- [ ] Forwarded-NPDU origin remains distinct from its BBMD peer through final
  SADR and reply routing.
- [ ] Destination/function mismatch, missing packet metadata and UDP truncation
  fail closed before routing.
- [ ] B/IP bursts cannot allocate an unbounded router backlog ahead of MS/TP.
- [ ] Standard MS/TP accepts exactly 501-byte final NPDUs and rejects 502 bytes
  before queue mutation.
- [ ] Both supported bare targets compile with no hosted dependencies.

### R5 — Diagnostics, operational control and hosted convergence

- [ ] Expose global and per-port accepted, full, oversize, expired, malformed,
  duplicate, policy-drop, rejection and transport-failure counters.
- [ ] Define counter saturation and reset semantics.
- [ ] Add immutable snapshots for routes, cache occupancy, port capabilities,
  queue outcomes and most recent failure reason.
- [ ] Add explicit controls for learned-route clear/refresh, port enable/disable,
  discovery announcement and diagnostics reset.
- [ ] Ensure control operations are synchronous, bounded and safe when invoked
  concurrently by an adapter that serializes core ownership.
- [ ] Share wire fixtures with the hosted router and record any intentionally
  different policy.
- [ ] Decide whether hosted `bacnet-network` should adapt to the core in a later
  PR; do not couple the embedded release to that refactor.
- [ ] Benchmark bounded admission latency and memory across representative
  route/cache/queue profiles.

Exit criteria:

- [ ] Every drop/refusal path increments one documented counter and exposes no
  packet data or stale mutable reference.
- [ ] Control commands have deterministic success/capacity/invalid-state results.
- [ ] Snapshot and counter reads do not allocate in the core.
- [ ] A documented memory budget includes route storage, duplicate cache,
  action scratch and the connected MS/TP queue profile.

## Extended MS/TP prerequisite (X0–X5)

Frame types `0x20` (Data Expecting Reply) and `0x21` (Data Not Expecting Reply)
use COBS and CRC-32K and are currently rejected deliberately. Types
`0x22..=0x7f` share the extended framing range but must not automatically be
treated as BACnet NPDU frames.

This is a separate local-fork stream and blocks any broad modern-router claim.
It may proceed alongside R1–R3, but its storage/capability contract must converge
with R4.

### X0 — Normative source and independent vectors

- [ ] Restore or otherwise obtain the configured Standard 135-2020 reference.
- [ ] Confirm exact extended Length, maximum-size, CRC residual and type rules.
- [ ] Check in independently derived COBS/XOR and CRC-32K vectors, including
  254-byte block boundaries, zero-heavy payloads and embedded `0x55` values.

### X1 — Portable COBS and CRC-32K primitives

- [ ] Add allocation-free caller-buffered primitives with exact capacity
  reporting and no partial-success ambiguity.
- [ ] Reject malformed code octets, bad CRC and output exhaustion.

### X2 — Extended one-shot and streaming codec

- [ ] Name only `0x20` DER and `0x21` DNER as BACnet extended data frames.
- [ ] Add caller-buffered or in-place decode; the immutable borrowed standard
  decoder cannot return transformed COBS payload bytes.
- [ ] Cover 501/502 and 1497/1498 decoded boundaries, every stream split point,
  corruption, resynchronization and mixed standard/extended streams.
- [ ] Extend fuzzing beyond the current one-shot standard decoder.

### X3 — MS/TP core storage and state machine

- [ ] Add an explicit standard/extended storage profile rather than globally
  widening every inline action slot.
- [ ] Update queues, action rings, receive delivery, reply storage, token-send
  scratch and frame-type selection together.
- [ ] Reject unsupported peer/port capability before queue mutation.
- [ ] Measure the memory delta for each supported profile.

### X4 — Hosted compatibility and timing

- [ ] Update hosted assembler limits, complete-frame timeout calculations,
  queue wrappers and compatibility decoding.
- [ ] Retain every existing standard-frame golden vector unchanged.
- [ ] Add mixed standard/extended in-memory serial tests at the slowest supported
  baud rate.

### X5 — Router composition and claim gate

- [ ] Report an explicit extended-capable per-port NPDU limit to the router.
- [ ] Prove 502-byte and 1497-byte NPDU queue→frame→decode→delivery paths.
- [ ] Prove deterministic 1498-byte rejection without queue mutation.
- [ ] Test mixed segments where an extended-capable local router cannot assume
  every remote station accepts extended frames.
- [ ] Record interoperability evidence before changing public support claims.

Simply widening `MAX_NPDU_LEN` or adding codec support is insufficient: current
core action rings and transmission scratch storage remain standard-frame sized.

## R6 — Release evidence and claim decision

- [ ] Complete the Clause 6 negative/transition matrix for implemented scope.
- [ ] Update the conformance ledger with exact source and test anchors.
- [ ] Run the full workspace, MSRV, bare-target, feature-matrix and packaging
  gates applicable to changed crates.
- [ ] Capture raw benchmark artifacts for admission latency, queue saturation
  and memory profiles if performance claims are made.
- [ ] Document unsupported BBMD/foreign-device/NAT/B/IP-M/PTP/security-network-
  message behavior explicitly.
- [ ] Publish no broad modern-router claim until X0–X5 are complete.

Claim milestones:

- R0–R3: portable routing core under active development; no deployable-router
  claim.
- R0–R5: library-complete B/IP↔standard-MS/TP router boundary, pending downstream
  firmware integration evidence.
- R0–R6 without X0–X5: standard-frame router only; Extended MS/TP explicitly
  unsupported.
- R0–R6 plus X0–X5: eligible for a broader claim after ledger and interop review.

## Deferred/out of scope

- ESP-IDF task creation, socket lifecycle, UART ownership, flashing and HIL
  orchestration belong to the firmware/integration team.
- BBMD/BDT/FDT, foreign-device registration, NAT rewriting and B/IP-M are not
  implied by ordinary B/IP routing.
- BACnet/SC, Ethernet, ARCNET, PTP and other data links are not part of this
  embedded B/IP↔MS/TP component.
- Routers do not perform APDU segmentation, NPDU fragmentation or silent
  truncation to fit an egress link.

## Verification gates

- `cargo test -p bacnet-router-core --locked`
- `cargo clippy -p bacnet-router-core --all-targets -- -D warnings`
- bare-target checks for `thumbv7em-none-eabi` and
  `riscv32imac-unknown-none-elf`
- embedded dependency-tree check proving no hosted runtime dependencies
- repository 700-line Rust source-file cap
- existing `bacnet-network`, `bacnet-mstp-codec` and `bacnet-mstp-core` focused
  regression suites after each integration slice

Normative caveat: the configured `_spec/` Standard 135-2020 PDF is absent from
this checkout. Existing Clause 6 ledger anchors and hosted tests inform the
work, but exact clause wording and Extended MS/TP length rules must be checked
against the Standard before raising conformance status.
