# Embedded BACnet/IP↔MS/TP router workplan

Status: active closeout

Last updated: 2026-10-11

Target branch: `fix/sc-router-review-closeout`

Scope: portable Rust library components. ESP-IDF task/socket/UART ownership,
firmware integration, flashing, and hardware-in-the-loop validation belong to
the downstream firmware team.

## Completion definition

The library work is ready for human review when:

1. the portable router discovers, learns, ages, controls, and forwards routes
   without unbounded storage or hosted-runtime dependencies;
2. standard and explicitly configured Extended MS/TP ports enforce their real
   NPDU limits before queue mutation;
3. unicast, routed broadcast, router-control, refusal, oversize, saturation,
   and duplicate-suppression behavior has exact or transition-level tests;
4. all changed crates pass their host, feature, bare-target, lint, file-size,
   and packaging gates; and
5. the conformance ledger and public limitations describe only the evidence
   actually present.

Independent interoperability and HIL evidence are not library completion
criteria, but remain claim/release gates and must be reported as downstream
work rather than inferred from in-memory tests.

## Architecture and fixed invariants

The hosted `bacnet-network` router remains the Tokio runtime implementation.
The embedded path is split between allocation-free `bacnet-router-core`,
bounded `bacnet-router-adapters`, and the caller-clocked `bacnet-mstp-core`.
The library owns no socket, UART, executor, lock, or unbounded queue.

- `AdmissionOutcome::AcceptedOwned` means the selected data-link queue or sink
  owns the bytes before returning.
- Validate the final rewritten NPDU against the selected port's effective
  limit. Never truncate or fragment in the router.
- Keep configured direct routes independent of learned-route capacity.
- Preserve the immediate UDP peer, actual local destination/group, and logical
  Forwarded-NPDU origin as distinct values.
- Preserve the final DNET/DADR across learned next hops; a next-hop address is
  a link-layer egress destination, not a replacement routed destination.
- Reserve network-control capacity separately from ordinary data capacity in
  every concrete adapter contract.
- Parse a complete control payload before mutating routing state.
- Default broadcast forwarding and Extended MS/TP peer capability fail closed.
- Use caller-provided monotonic time for expiry, aging, and duplicate windows.
- Enforce the repository's 700 non-empty/non-comment Rust-line limit.

## Phase dashboard

“Implemented” means code exists on the branch. “Complete” additionally means
the phase exit criteria and recorded gates pass. Uncommitted closeout work is
never counted as complete.

| Phase | State | Remaining gate |
|---|---|---|
| R0 Portable boundary/direct routing | Complete | None |
| R1 Unicast routing/refusal | Complete | Discovery-before-refusal and zero-hop silence landed in `8a0c71fb` |
| R2 Discovery/table control | Complete | Canonical Initialize query landed in `60ffb85b`; router-core matrix passes |
| R3 Routed broadcasts/loop safety | Complete | Host, strict-lint, feature, and bare-target gates pass |
| R4 Standard B/IP↔MS/TP adapters | Complete | Composition and both bare-target gates pass |
| R5 Diagnostics/operational control | Complete | Exactly-once observers and bounded benchmark evidence pass |
| X0 Normative sources/vectors | External gap | Licensed consolidated 135-2020 reconciliation |
| X1 COBS/CRC-32K primitives | Complete | None |
| X2 Extended codec | Complete | Independent fixtures and streaming fuzz landed in `8a3f0dd0` |
| X3 Extended core profile | Complete | Explicit profiles, memory evidence, and strict gates pass |
| X4 Hosted extended framing | Complete | Framing/assembly only; hosted application delivery is not claimed |
| X5 Extended router composition | Complete | Composition and boundary gates pass; interop remains a claim gate |
| R6 Release evidence/claim | In review | Independent review and documented packaging caveat |

## Completed implementation baseline

The following work is present and should not be reopened unless a closeout
test identifies a regression:

- R0: strict borrowed BVLL ingress, bounded direct/learned partitions,
  direct-network rewrite, final-NPDU limits, and synchronous owned admission.
- R1/R2: shared strict NPDU parsing, learned-route forwarding, source-aware
  rejection actions, network-control codecs, Who-Is/I-Am handling,
  Initialize-Routing-Table transactions, owner-bound Busy/Available state,
  caller-time route aging, and detached route snapshots.
- R3: explicit remote/global broadcast policy, exact header rewrites,
  deterministic per-port fanout, bounded duplicate suppression, ingress
  exclusion, reserved control admission, and fail-closed cache saturation.
- R4: bounded B/IP ingress/egress contracts, standard MS/TP queue ownership,
  priority preservation, typed backpressure, reverse MS/TP→B/IP delivery,
  request bindings, and bidirectional in-memory composition tests.
- R5: bounded global/per-port diagnostics, snapshots, saturation/reset rules,
  learned-route and port controls, announcement requests, and representative
  memory-budget assertions.
- X1–X4: allocation-free COBS/XOR-`0x55`, CRC-32K, extended one-shot and
  streaming codecs, explicit standard/extended storage profiles, hosted
  framing/assembly/timing support, reserved-type rejection, and maximum-frame
  split tests. The hosted adapter does not deliver Extended MS/TP NPDUs to
  applications; end-to-end Extended delivery belongs to the explicit embedded
  profile and router adapter.
- X5 foundation: standard and extended MS/TP egress profiles, fail-closed peer
  capability, 501/502/1497/1498 queue limits, and extended frame composition.

Primary implementation commits include `d39425ea`, `902a85b2`, `7f192fe1`,
`647800ed`, `dec02bff`, `d91db207`, `47669cd5`, `8d8767a5`, `9337d020`,
`afc15d1a`, `4fef1f9e`, `a592bbca`, `d1d2dda2`, `c647b807`, `dbd93ae5`,
`8a0c71fb`, `f7385399`, `8a3f0dd0`, `d0f13836`, `ec5046f3`, and
`321c7cce`.

## Remaining phases

### C1 — Finish the active router-control closeout

Purpose: make unknown-route behavior discovery-first and make the control wire
format unambiguous before taking a clean checkpoint.

- [x] Finish and commit the bounded `RouteDiscovery` integration (`8a0c71fb`).
- [x] First unknown destination returns one Who-Is-Router action; repeat calls
  before the caller-time deadline report pending without duplicating traffic.
- [x] A timed-out discovery produces a source-aware
  Reject-Message-To-Network; capacity exhaustion remains distinct and
  observable (`29498835`).
- [x] A hop-exhausted NPDU is dropped without generating another network
  message.
- [x] Encode an Initialize-Routing-Table query as a single zero
  Number-of-Ports octet and reject an empty request body as truncated
  (`60ffb85b`).
- [x] Prove malformed discovery/control input and every truncation offset leave
  route state unchanged (`f006cad9`).
- [x] Run router-core tests, strict Clippy, `git diff --check`, and the file-size
  gate for the discovery/refusal checkpoint.
- [x] Repeat those gates after the Initialize-Routing-Table correction and
  commit the complete control checkpoint.

Exit evidence:

- exact actions for first request, pending request, timeout, and full pending
  table;
- exact refusal destination for direct and learned source contexts;
- no egress/refusal for hop count zero; and
- canonical query and zero-entry ACK remain distinct.

### C2 — Close Extended MS/TP evidence and composition

Purpose: turn the current implementation into evidence-backed end-to-end
support without treating every `0x20..=0x7f` frame as an NPDU.

- [x] Add the complete official Annex X.4 DNER wire fixture as an independent
  decoder/encoder oracle, with source attribution (`8a3f0dd0`).
- [x] Add independently constructed valid-CRC receive fixtures for decoded
  lengths 501, 502, 1497, and 1498. Confirm 502 and 1497 delivery and
  deterministic out-of-range handling for 501 and 1498 (`8a3f0dd0`).
- [x] Retain negative coverage for all reserved types `0x22..=0x7f`, malformed
  encoded CRC lengths, bad CRC, noncanonical COBS, truncation,
  resynchronization, and mixed standard/extended streams.
- [x] Keep the extended streaming decoder in the fuzz target (`8a3f0dd0`).
- [x] Add an explicit `PortKind::MstpExtended` with a 1497-byte hard ceiling.
  Do not model extended capability as `Other` or by merely raising a standard
  port's configured length (`f7385399`).
- [x] Update address validation, forwarding, broadcast selection, snapshots,
  composition checks, and tests for the new kind (`f7385399`, `697fe5c7`).
- [x] Export the extended core profile required by the adapter (`07c0fb82`).
- [x] Correct the extended delivery tests so their receiver station matches
  the encoded destination, then prove queue→frame→decode→delivery at 502 and
  1497 bytes (`07c0fb82`).
- [x] Generalize request/reply bindings over the standard and extended core
  profiles and prove an Extended DER delivery can complete or time out through
  the public composition seam (`f7385399`).
- [x] Prove an extended-capable port still rejects an unverified extended peer
  and rejects 1498 bytes before queue mutation (`f7385399`).
- [x] Record the measured standard-versus-extended core memory delta: standard
  profile 13,368 bytes, extended profile 37,432 bytes, delta 24,064 bytes
  (`86547c56`).
- [x] Accept valid slave source addresses 128–254 for standard and extended
  frames while continuing to reject source 255 (`da7a4eda`).
- [x] Reset and resynchronize the standard profile after a valid Extended
  frame exceeds its configured receive storage (`da7a4eda`).
- [x] Derive the default transmit watchdog from maximum Extended-frame wire
  time at the configured baud rate and reject undersized profile watchdogs
  (`da7a4eda`).
- [x] Deliver Extended DNER while waiting for a reply and compare both encoder
  and decoder output against the complete Annex X.4 fixture (`da7a4eda`).

Exit evidence:

- codec, core, hosted framing/assembly, and router-adapter suites all pass;
- standard golden vectors are unchanged;
- standard ports remain capped at 501 bytes regardless of configured values;
- extended ports are capped at 1497 bytes and require explicit peer opt-in;
- both supported bare targets compile.

The hosted `bacnet-transport` seam is evidence for framing, stream assembly,
timing, and resynchronization only. It still drops Extended frames before
application delivery, so this plan does not claim hosted Extended MS/TP
endpoint support.

External X0 gate: official addenda support the implementation and are recorded
in `docs/plans/evidence/extended-mstp-source-dossier.md`. The configured
licensed consolidated Standard 135-2020 PDF is absent. Obtain and reconcile
Clauses 9.3, 9.5, 9.6, 9.10 and Annexes G/X before raising conformance status;
do not substitute an unofficial copy. This does not block finishing the public
addendum-derived tests.

### C3 — Operational and performance evidence

Purpose: demonstrate that the bounded design remains predictable under the
route, cache, and queue profiles a firmware consumer will actually configure.

- [x] Audit that every drop/refusal/admission failure increments exactly one
  documented counter and retains no packet data or mutable borrowed state.
  Typed core observers landed in `d0f13836`; concrete observed composition
  landed in `ec5046f3`.
- [x] Confirm control commands have deterministic success, capacity, and
  invalid-state results.
- [x] Add a reproducible local benchmark for direct forwarding, learned
  forwarding, broadcast fanout, duplicate lookup, and saturated queue
  admission across representative table sizes (`321c7cce`).
- [x] Capture raw host/toolchain/configuration output; do not make latency or
  throughput claims from a single workstation run (`321c7cce`).
- [x] Record memory budgets for routes, pending discovery, duplicate cache,
  action scratch, diagnostics, standard queues, and extended queues
  (`321c7cce`).

Exit evidence: benchmark command and raw artifact are reproducible, memory
limits are explicit, and no performance claim exceeds the recorded evidence.

### C4 — Release evidence and documentation

Purpose: produce a reviewable library checkpoint without broadening protocol
or platform claims.

- [x] Update `BACNET-6-ROUTER-MESSAGES` in the Standard 135-2020 conformance
  ledger with current source/test anchors and remaining Clause 6 gaps.
- [x] Update the MS/TP ledger evidence for standard and extended frame scope.
- [x] Document unsupported BBMD/BDT/FDT, foreign-device, NAT, B/IP-M, PTP,
  security-network-message, and non-B/IP/non-MS/TP behavior.
- [x] Record that actual ESP-IDF UDP destination metadata, socket lifecycle,
  UART direction control, timers, task serialization, and HIL remain
  downstream responsibilities.
- [x] Run the full verification matrix below and retain exact command/results.
- [ ] Run a BACnet-focused correctness/safety review and a release-readiness
  review. Resolve blockers/high findings or record an explicit human decision.
- [ ] Package the branch for human review as focused commits; do not publish a
  broad modern-router claim until the external X0 and interoperability gates
  are satisfied.

### C5 — Resolve independent review findings

Purpose: close the concrete correctness and safety findings raised by the
BACnet-focused review panel before asking the human team to review the branch.

- [x] Route all direct unicast APIs with a broadcast link destination to the
  explicit broadcast-policy seam; neither B/IP→MS/TP nor MS/TP→B/IP can bypass
  policy and duplicate suppression (`9f8e7892`).
- [x] Represent direct delivery and learned-router next hops explicitly in
  `BroadcastTarget`, so remote broadcasts to learned networks remain inside
  policy/duplicate suppression while preserving DNET and decrementing hop
  count (`63d00b5d`).
- [x] Reject configured subnet broadcast, limited-broadcast, and IPv4
  multicast addresses supplied through the B/IP unicast link-address path
  before BVLL encoding or sink mutation (`63d00b5d`).
- [x] Discard routed NPDUs with hop count zero or one before egress, cache
  insertion, discovery, or refusal generation (`9f8e7892`).
- [x] Reclaim expired discovery slots before reporting bounded-capacity
  exhaustion (`9f8e7892`).
- [x] Suppress Who-Is-Router responses for direct and learned routes whose
  egress is the request ingress (`9f8e7892`).
- [x] Encode refusals to a local ingress source as local network messages and
  retain routed addressing only for routed sources (`9f8e7892`).
- [x] Validate broadcast DADR/class combinations before cache or fanout and
  cap router-control payload processing at 1497 bytes (`9f8e7892`).
- [x] Close the Extended receive, watchdog, source-address, reply-delivery,
  and Annex X.4 oracle gaps listed in C2 (`da7a4eda`).
- [ ] Re-run the correctness, Clause 6, data-link, safety, test, and release
  reviews against the replacement commit and resolve every blocker/high
  finding.

Initialize-Routing-Table query, ACK, and transactional parsing are covered.
The precise semantics of non-empty request entries and their wire `port_id`
remain a consolidated-standard review item; the portable core currently treats
the peer port identifier as opaque and learns ownership from the local ingress
port plus immediate peer. Do not promote that behavior to a conformance claim
until X0 is complete.

The direct-delivery interpretation of hop count one is also an X0 item. The
current implementation discards routed NPDUs at hop count zero or one before
any direct or learned delivery. Published errata and available conformance-test
material support that behavior, but a later public-review draft creates enough
ambiguity that the consolidated 135-2020 text must be checked before promoting
the behavior to a conformance claim.

## Remaining external and release phases

These phases intentionally remain open after the portable library is ready for
human review:

1. **X0 consolidated-standard reconciliation:** obtain the licensed
   Standard 135-2020 text and reconcile Clause 6 routing-table semantics plus
   Clauses 9.3, 9.5, 9.6, 9.10 and Annexes G/X. Update tests and ledger rows for
   any difference; do not infer consolidated wording from addenda.
2. **I1 independent interoperability:** exercise B/IP↔standard MS/TP and
   explicitly enabled Extended MS/TP with an independent implementation,
   including discovery, table initialization, broadcasts, refusal paths,
   saturation, 501/502/1497/1498 boundaries, and slave sources 128–254.
3. **F1 downstream ESP-IDF integration/HIL:** the firmware team owns UDP
   ancillary destination metadata, socket lifecycle, UART direction and drain,
   monotonic timers, task serialization, device diagnostics, flashing, and HIL.
4. **P1 publication readiness:** verify the exact declared MSRV, publish
   dependency crates in order (`types` → `codec` → `mstp-core`; `router-core`
   independently; adapters and transport after their dependencies), prepare
   every package against the private registry, then build and smoke-test the
   tagged artifacts. The current admission benchmark is a historical baseline
   measured at `ec5046f3`, not a performance claim for later closeout commits.

## Verification matrix

Minimum local gates:

```text
cargo test -p bacnet-router-core --locked
cargo test -p bacnet-router-adapters --all-features --locked
cargo test -p bacnet-mstp-codec --all-features --locked
cargo test -p bacnet-mstp-core --all-features --locked
cargo clippy -p bacnet-router-core --all-targets --no-deps --locked -- -D warnings
cargo clippy -p bacnet-router-adapters --all-targets --no-deps --locked -- -D warnings
cargo clippy -p bacnet-mstp-codec --all-targets --all-features --no-deps --locked -- -D warnings
cargo clippy -p bacnet-mstp-core --all-targets --all-features --no-deps --locked -- -D warnings
.github/scripts/check-file-size.sh
git diff --check
```

Also run:

- no-default-feature checks for the router, adapter, codec, and core crates;
- `thumbv7em-none-eabi` and `riscv32imac-unknown-none-elf` checks using the
  pinned Rust 1.97.1 toolchain/compiler when the default Homebrew Cargo cannot
  locate the installed target `core`;
- focused regressions for `bacnet-network` and hosted `bacnet-transport` after
  control-codec or hosted extended-frame changes;
- package-list/content checks for every changed publishable crate; and
- a refreshed `ccc` index after the final source/document changes.

Closeout result: all listed host tests, strict Clippy commands, feature checks,
and both pinned bare-target checks passed. Package content listing passed for
all five changed publishable crates, and `bacnet-router-core` also passed
standalone package verification. Cargo cannot prepare the four packages that
depend on unpublished `0.11.8` sibling crates against crates.io; this is a
release-order/registry prerequisite rather than a source-content failure and
must remain visible to the human release review. Exact commands, toolchains,
results, and claim limits are recorded in
`docs/plans/evidence/embedded-router/closeout-2026-10-11.md`.

## Claim gates and downstream handoff

| Milestone | Permitted statement |
|---|---|
| C1 complete | Portable routing/control closeout is ready for consolidated verification |
| C1–C5 complete, without external X0/interoperability | Library-complete B/IP↔MS/TP implementation; conformance/interoperability pending |
| C1–C5 plus X0 reconciliation and independent interop | Eligible for human review of a broader modern-router claim |
| Firmware/HIL complete | Platform deployment evidence, maintained by the firmware team |

The library handoff must expose bounded ingress/egress contracts and their
timing/capacity requirements. It must not absorb firmware task creation,
Tailscale/host setup, ESP-IDF socket ownership, UART ownership, flashing, or
field-network validation into this workplan.
