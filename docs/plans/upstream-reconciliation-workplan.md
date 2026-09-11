# Upstream reconciliation work plan

Status: in progress
Created: 2026-09-11
Local source: `dev` at `bf6922d5efd92da3890c4ede2fbf76dee8c8abb7`
Upstream target: `upstream/dev` at `a62821b5f961281663b373b901135a8cd8d4ff08`
Merge base: `6b9ac4f0f2dfadbf6dbbda88c46ad6a0b0aa43d2`
Related plans:
`docs/plans/ace-ng-edge-python-parity-plan.md` and
`docs/plans/bacpypes3-feature-parity-workplan.md`

## Goal

Rebase ACE IoT's differentiated runtime, Python, topology, BBMD-management,
simulation, and interoperability capabilities onto current upstream without
discarding upstream's post-0.10.1 correctness, security, resource-bound, and
conformance work.

The result must preserve the repository-owned capability contracts already
proved by the parity plans while adopting upstream 0.11.0 behavior as the
default implementation wherever the two lines solve the same lower-level
problem.

This plan does not authorize changes to sibling repositories, new CI workflows,
a production migration, or broader public conformance claims.

## Baseline assessment

The branches have diverged too far for conflict count or commit ancestry to be
used as a correctness argument:

| Measure | Local `dev` | Current `upstream/dev` |
|---|---:|---:|
| Commits after the merge base | 20 (19 non-merge) | 352 |
| Changed files after the merge base | 276 | 948 |
| Insertions / deletions | +53,330 / -6,901 | +194,496 / -17,171 |
| Rust test markers at the branch tip | approximately 2,624 | approximately 4,503 |
| Python test methods at the branch tip | approximately 39 | approximately 32 |

`git cherry -v upstream/dev dev` reports all 19 local patches as unique. A
three-way merge trial reports 55 textual conflicts, concentrated in transport,
server, client, and Python binding code. Local has 86 test-like files absent
upstream; upstream has 322 absent locally.

Conclusion: no local W1-W13 workstream is wholly superseded, but portions of
the BBMD, COV, TSM, segmentation, SC, and request-lifecycle implementations
must be replaced by their newer upstream equivalents.

## Working rules

- Start the reconciliation line from the pinned upstream target, not from a
  conflict-resolved merge into the old local tree.
- Keep each PR to one BACnet layer, API boundary, state-machine family, or
  measured hotspot.
- Preserve provenance by naming the local source commits and upstream commits
  used in every PR description.
- Port behavior and tests, not obsolete implementation structure.
- Prefer upstream transaction ownership, admission, resource limits, and
  fail-closed defaults unless a documented local contract requires an
  extension.
- Do not restore the legacy invoke-ID-only cross-subnet TSM fallback.
- Do not weaken upstream quotas, deadlines, validation, response correlation,
  SC identity requirements, or shutdown ownership to make local tests pass.
- Add or transplant the focused tests for a capability in the same PR as that
  capability. A historical evidence document is not a substitute for a new
  test run.
- Preserve numeric proprietary object/property identifiers, B/IP ports, raw
  MAC bytes, DNET/DADR, ingress attachment identity, and BVLL origin metadata.
- Exercise an installed Python wheel for Python acceptance.
- Do not add new CI workflows unless explicitly requested.
- Do not broaden README, PICS, BIBB, or public support claims without updated
  conformance ledger rows and current evidence.
- Leave `.claude-flow/`, `.hive-mind/`, and unrelated user changes untouched.

## Architectural decisions

| Boundary | Decision | Local behavior to preserve | Upstream behavior to adopt |
|---|---|---|---|
| Branch foundation | Reconstruct on upstream | Local commit provenance and tests | Complete upstream 0.11.0 tree and history |
| Confirmed transactions | Upstream owns correlation and lifecycle | Typed direct/routed targets and stable Python errors | `bacnet-endpoint-core`, canonical peer identity, coordinated TSM, routed path limits |
| Segmentation | Upstream owns the state machines | Routed target behavior and error projection | Current receive/send admission, cleanup, retry, and sequence-space rules |
| BBMD core | Upstream owns wire admission and fanout safety | Live control, snapshots, arbitrary policy extension, Python API | Fail-closed management, bounded FDT/BDT, rate limits, deduplication, amplification budgets |
| COV server | Upstream owns notification scheduling | Python object metadata, interoperability semantics, local/runtime observation | Per-peer quotas, work budgets, exact deltas, confirmed-worker ownership |
| BACnet/SC | Upstream owns transport/security behavior | Runtime attachment configuration and health projection | Current TLS/UUID/admission/heartbeat/deadline behavior |
| Runtime aggregate | Local runtime remains a separate crate | Reconcile, batching, cache, scan, health, resource accounting, event aggregation | Use upstream endpoint/transaction seams rather than parallel ownership |
| Python API | Preserve the union of both public surfaces | W1-W13 and Rust-runtime APIs | Upstream 0.11 audit, file, DCC, SC, admission, timestamp, and object APIs |
| Documentation | Reconcile claim-by-claim | Parity evidence and repository-owned limitations | Current upstream changelog, conformance ledger, API docs, and version |

## Definition of reconciliation-ready

Reconciliation is ready for `dev` only when all of the following are true:

- All upstream tests and workspace members present at the pinned target remain
  present unless a removal has an explicit decision record.
- The installed Python surface is a reviewed union: no unexplained removal from
  either the local or upstream stub manifests.
- W1-W13 repository-owned acceptance behavior passes on the reconciled source.
- Upstream transaction, BBMD, COV, SC, admission, and resource-limit tests pass
  without weakening their assertions.
- Direct and routed traffic preserves complete peer identity, including
  non-default B/IP ports and multi-byte DADR.
- Runtime attachment add/update/remove/rollback and shutdown return tasks,
  sockets, descriptors, serial handles, and registries to their accepted
  baselines.
- W12 campus routing and W13 four-way cross-stack fixtures pass from a clean,
  newly built wheel.
- Conformance ledgers, API documentation, stubs, changelog, and version metadata
  describe the combined implementation and its remaining limitations.
- The old local `dev` tip remains recoverable by a permanent tag or branch until
  the reconciled line has passed the complete release gate.

## Phase 0 - freeze inputs and create the evidence ledger

Purpose: make accidental feature or test loss visible before code movement.

- [x] Create permanent refs for the pinned local tip, upstream target, and merge
      base.
- [x] Record `git status`, remote URLs, branch tips, tag tips, and toolchain
      versions.
- [x] Generate sorted manifests for:
      - workspace members and Cargo features;
      - Rust public exports;
      - Python classes, functions, methods, signatures, and exceptions;
      - test files and test names;
      - conformance rows and public support statements.
- [x] Classify every local-only file as `port`, `replace-with-upstream`,
      `evidence-only`, `obsolete`, or `needs-design`.
- [x] Create `docs/plans/evidence/upstream-reconciliation-ledger.md` with one
      row per local commit/workstream and fields for upstream substitute,
      destination PR, tests, and disposition.
- [x] Mark the legacy March branches as frozen; do not cherry-pick them into the
      reconciliation line.

Acceptance:

- The manifests reproduce the recorded branch counts and can be diffed after
  every tranche.
- Every one of the 19 local non-merge commits has a planned disposition.
- No branch deletion or force-push has occurred.

## Phase 1 - establish the upstream-first integration line

Purpose: prove the upstream target before adding local behavior.

- [x] Create the integration branch from the pinned `upstream/dev` commit.
- [x] Preserve local agent guidance, reconciliation plans, and evidence files
      in a documentation-only commit.
- [ ] Run upstream's formatter, workspace checks, tests, Clippy, security, and
      feature-matrix commands unchanged.
- [x] Build and install the unmodified upstream wheel in a clean environment;
      record its API manifest and Python test result.
- [ ] Record platform or environmental skips separately from failures.

Acceptance:

- The pinned upstream source passes its own applicable gates.
- The branch remains behaviorally identical to upstream apart from local
  guidance, plans, and evidence.
- Any pre-existing upstream failure is documented before local code is added.

## Phase 2 - packaging and Python contract harness

Purpose: establish a stable test boundary without porting product features yet.

- [x] Apply the maturin-only `pyo3/extension-module` configuration without
      reverting upstream dependency versions or workspace members.
- [x] Port the installed-package harness and API-manifest comparison tooling.
- [x] Split the local Python contract assertions by capability so absent later
      tranches are explicit expected gaps rather than blanket skips.
- [x] Add a union-surface report comparing old local, pinned upstream, and the
      integration branch.

Acceptance:

- `cargo check --workspace --locked` works outside a Python interpreter.
- A clean wheel builds and imports.
- The upstream Python surface has no unexplained removals.
- Later local capabilities appear as enumerated open contract rows.

## Phase 3 - typed targets and client operations

Purpose: restore persisted direct/routed operations on upstream transaction
ownership.

- [x] Port `DirectTarget` and `RoutedTarget`, including byte/range validation.
- [ ] Adapt RP, RPM, WP, WPM, file, device-management, and COV request entry
      points to upstream's canonical logical-peer/transaction model.
- [ ] Preserve non-default B/IP ports, DNET, multi-byte DADR, array indexes,
      write priority, and response order.
- [x] Port Python timeout, protocol, Reject, Abort, BVLC, network-reject, and
      per-property error projection.
- [ ] Recreate routed segmentation coverage using upstream state machines.
- [ ] Explicitly reject any design that matches a routed response by invoke ID
      alone.

Acceptance:

- A fresh client with no discovery state completes direct and routed
  RP/RPM/WP/WPM through a persisted target.
- Upstream response-correlation, routed-reply, path-limit, TSM, and segmentation
  suites remain green.
- Injected malformed, stale, wrong-peer, and ambiguous responses cannot complete
  the wrong transaction.

## Phase 4 - discovery, BVLL management, and managed COV client APIs

Purpose: restore the remainder of the original P0 Python parity boundary.

- [x] Port scoped/unscoped Who-Is-Router-To-Network collection and immutable
      `RouterInfo` snapshots.
- [x] Port ephemeral-socket Python Read-BDT/Read-FDT with typed BVLC errors,
      cancellation cleanup, and exact IP/port/mask/TTL projection.
- [x] Port managed direct/routed COV subscription renewal, cancellation, lag
      reporting, and event iteration.
- [x] Preserve upstream's existing unsolicited COV ACK policy and routed ACK
      behavior.

Acceptance:

- Router discovery merges duplicate claims without losing responder address
  bytes.
- Repeated and cancelled BDT/FDT probes release sockets and correlation state.
- Confirmed and unconfirmed COV delivery, renewal, expiry, cancellation, and
  shutdown pass for direct and routed targets.

## Phase 5 - Rust-owned multi-attachment runtime

Purpose: reintroduce the unique aggregate runtime without duplicating upstream
transaction ownership.

- [x] Add `bacnet-runtime` as a workspace member alongside, not instead of,
      `bacnet-endpoint-core`.
- [x] Port attachment configuration, registry, reconcile/rollback, supervisor,
      scheduler, batching, cache, scan, catalog, topology, health, and resource
      accounting modules.
- [x] Refactor runtime request dispatch to call upstream endpoint/client seams.
      Do not retain a second transaction coordinator.
- [x] Restore the coarse Python `BACnetRuntime` API and immutable result models.
- [x] Restore B/IP, MS/TP, and SC attachments using current upstream transport
      constructors and security requirements.
- [ ] Preserve attachment-scoped device, route, COV, health, and event ownership.

Acceptance:

- All seven runtime test groups pass after being reviewed for current upstream
  semantics.
- Reconcile is idempotent and rollback preserves the previous healthy revision.
- Partial transport failure does not stop unrelated attachments.
- Upstream SC identity/TLS/deadline rules remain enforced through Python and the
  runtime.

## Phase 6 - I-Am provenance and foreign-device lifecycle

Purpose: restore W1/W2 using upstream B/IP admission and endpoint flow.

- [x] Carry BVLC function, immediate UDP peer, Forwarded-NPDU originator, raw
      MAC, SNET/SADR, and attachment identity through receive metadata.
- [x] Add the duplicate-preserving bounded `IAmEvent` stream without changing
      upstream's merged discovery table semantics.
- [x] Project I-Am observations into runtime events and Python iterators with
      observable lag.
- [ ] Port managed foreign-device startup, TTL/2 renewal, NAK/expiry/recovery
      state, status handles, runtime health, and Python errors.
- [x] Use upstream's explicit and bounded BBMD foreign-device admission policy;
      do not restore unbounded FDT behavior.

Acceptance:

- Original and forwarded duplicate I-Am messages remain individually observable
  with byte-exact provenance.
- Foreign registration transitions through pending, registered, rejected,
  expired, and recovered states without blocking other attachments.
- Broadcast delivery through a BBMD and recovery after BBMD restart pass in a
  clean-wheel fixture.

## Phase 7 - B/IP socket isolation and BBMD control

Purpose: preserve W3/W11 while treating upstream safety behavior as invariant.

- [x] Reintroduce opt-in SO_REUSEPORT and interface isolation behind a narrowly
      scoped B/IP socket configuration API.
- [ ] Revalidate Linux and macOS same-port/wildcard/subnet-broadcast matrices
      against upstream's current UDP metadata and receive path.
- [ ] Port live BBMD control, BDT replacement/persistence, FDT snapshots,
      management ACLs, counters, and Python management objects.
- [ ] Implement the arbitrary BVLL policy hook as an extension before normal
      BBMD processing, while retaining upstream validation, rate limits,
      capacity limits, fanout deduplication, amplification budgets, and
      fail-closed defaults.
- [ ] Define precedence when the custom policy, management policy, foreign-device
      policy, or fanout policy disagree. Denial must win unless the Standard and
      the conformance ledger justify a narrower rule.
- [ ] Keep Python policy callbacks bounded by queue capacity, timeout, failure
      threshold, and cooldown; native policy remains the high-rate path.

Acceptance:

- Default socket exclusivity is unchanged; opt-in sharing never crosses
  interface attachment identity.
- All upstream BBMD safety and response-amplification tests remain green.
- Policy deny/drop/reject/cut-through behavior emits at most one copy per
  destination and cannot bypass upstream admission limits.
- Live BDT/FDT mutation and snapshots agree with wire Read-BDT/Read-FDT results.

## Phase 8 - virtual networks and composable router

Purpose: restore W5 on current router and endpoint boundaries.

- [x] Port the named N-node `VirtualNetwork` transport with explicit membership
      ownership and bounded delivery.
- [x] Adapt `BACnetRouter` composition to upstream router control and forwarding
      types.
- [x] Preserve duplicate-network rejection, route learning, IARTN behavior,
      hop-count handling, and per-port counters.
- [x] Restore Python virtual server attachment and router lifecycle APIs.
- [x] Decide whether upstream endpoint-core can own each virtual endpoint
      directly; document any remaining adapter layer.

Acceptance:

- Two virtual networks route discovery, confirmed requests, errors, and COV
  through a B/IP-facing router.
- Router restart releases all memberships and tasks.
- Upstream router data-option, priority, hop-count, and rejection tests remain
  green.

## Phase 9 - server identity, live NetworkPort, COV metadata

Purpose: restore W4/W6 without replacing upstream server scheduling.

- [x] Port configurable Device identity with current upstream defaults and
      property validation.
- [ ] Bind NetworkPort objects to live B/IP/BBMD state through a read-only
      snapshot boundary plus explicit control operations.
- [x] Port Python object registration metadata: description, units, state text,
      number of states, COV increment, and supported writable present values.
- [ ] Express COV criteria through upstream object traits and notification
      engine; do not restore the local worker implementation.
- [ ] Reconcile PICS generation with runtime capabilities and upstream's current
      service/object capability derivation.

Acceptance:

- Third-party reads of Device and NetworkPort properties match live configured
  and BBMD state.
- Analog increments and binary/multi-state/character-string changes meet the
  documented COV criteria across the deterministic 1,000-update fixture.
- Upstream COV quota, work-budget, confirmed-worker, Life Safety, and shutdown
  tests remain green.

## Phase 10 - diagnostics and passive observation

Purpose: restore the low-coupling W7/W9/W10 surfaces after receive and runtime
paths stabilize.

- [x] Port public hinted raw-value decoding and structured tag description with
      adversarial bounds and malformed-input tests.
- [ ] Port the optional APDU observer with bounded queues, decode-failure
      projection, lag reporting, and clean shutdown.
- [ ] Restore passive confirmed/unconfirmed COV projection into runtime events
      with source and attachment provenance.
- [ ] Ensure observation cannot alter packet admission, transaction correlation,
      ACK policy, or forward progress.

Acceptance:

- Observer-disabled execution has no Python callback or per-packet crossing.
- Observer lag and malformed APDUs are visible without killing the client.
- Raw decode rejects truncated, overlong, nested, or type-mismatched inputs
  deterministically.
- Unsolicited COV reaches both direct Python and runtime consumers without
  duplicate protocol responses.

## Phase 11 - cross-stack, performance, and release evidence

Purpose: replace historical evidence with results from the reconciled source.

- [ ] Run W3 BBMD policy, W5 virtual router, W6 COV, and W12 campus fixtures.
- [ ] Run W13 in all four client/server pairings: bacpypes3/bacpypes3,
      rusty/bacpypes3, bacpypes3/rusty, and rusty/rusty.
- [ ] Re-run the runtime A/B benchmark with correctness-equivalent workloads and
      record latency, throughput, Python crossing count, memory, task, socket,
      descriptor, and cancellation results.
- [ ] Re-run the supported clean-wheel platform/interpreter matrix from one
      immutable source archive.
- [ ] Update conformance ledgers, plans, API docs, stubs, changelog, support
      summary, and limitations from the new evidence.
- [ ] Perform a release-readiness review before advancing `dev`.

Acceptance:

- No unexplained topology, property, error-category, COV, or resource delta
  remains in W13 normalized output.
- Performance claims include raw artifacts, baseline identity, workload,
  variance, and limitations.
- All published claims point to current tests and evidence produced from the
  reconciled commit.

## Proposed PR queue

| Order | PR | Primary ownership | Depends on |
|---:|---|---|---|
| 1 | Freeze manifests and reconciliation ledger | docs/evidence | none |
| 2 | Pin and verify upstream-first integration baseline | workspace | 1 |
| 3 | Maturin build boundary and installed contract harness | Python packaging | 2 |
| 4 | Typed direct/routed targets and property operations | client + Python | 3 |
| 5 | Router discovery and BDT/FDT Python management | network/client + Python | 4 |
| 6 | Managed COV and typed error projection | client + Python | 4 |
| 7 | Runtime core over upstream endpoint ownership | runtime | 4-6 |
| 8 | Runtime B/IP, MS/TP, and SC attachments | runtime/transports | 7 |
| 9 | I-Am provenance and event streams | transport/client/runtime + Python | 7-8 |
| 10 | Foreign-device lifecycle and health | B/IP/runtime + Python | 8-9 |
| 11 | B/IP interface isolation and SO_REUSEPORT | B/IP transport | 2 |
| 12 | BBMD live control and snapshots | B/IP transport + Python | 10-11 |
| 13 | BVLL policy hook over upstream safety policies | B/IP transport + Python | 12 |
| 14 | VirtualNetwork transport | transport | 7 |
| 15 | Composable router and Python lifecycle | network + Python | 14 |
| 16 | Device identity and live NetworkPort | objects/server + Python | 12 |
| 17 | Server COV metadata and interop fixture | objects/server + Python | 6, 16 |
| 18 | Raw decode, APDU observer, passive runtime COV | diagnostics/runtime + Python | 7, 9 |
| 19 | W12/W13 cross-stack acceptance | fixtures/evidence | 13, 15, 17, 18 |
| 20 | Performance, packaging, conformance, and release audit | evidence/docs | 19 |

PRs may be combined only when they remain within one testable boundary and do
not obscure whether upstream safety behavior was retained.

## Validation gates

### G0 - provenance and inventory

- [ ] Pinned refs and generated manifests exist.
- [ ] Every local commit and local-only test has a disposition.
- [ ] Every upstream-only workspace member and test family is accounted for.

### G1 - upstream baseline

- [ ] The pinned upstream target passes applicable checks before local code.
- [ ] Toolchain, feature flags, and environmental skips are recorded.

### G2 - per-PR Rust correctness

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo check --workspace --locked`
- [ ] Targeted crate tests for the affected boundary.
- [ ] Applicable upstream regression suites for the affected boundary.
- [ ] No test removal or assertion weakening without a written decision.

### G3 - combined workspace

- [ ] `cargo test --workspace --exclude rusty-bacnet --locked --no-fail-fast`
- [ ] Applicable feature matrix including `bacnet-types/serde`, IPv6, SC TLS,
      serial/MS/TP, and Ethernet.
- [ ] Clippy, audit, deny, MSRV, no-std, and file-size gates remain valid.

### G4 - installed Python contract

- [ ] Clean wheel build and install.
- [ ] Complete installed-package suite.
- [ ] Stub/runtime export and signature agreement.
- [ ] Reviewed union report for local and upstream Python APIs.

### G5 - transport and interoperability

- [ ] B/IP, MS/TP, SC, heterogeneous runtime, W3, W5, W6, and W12 fixtures.
- [ ] W13 four-way cross-stack comparison.
- [ ] Clean shutdown and resource return after success, timeout, cancellation,
      malformed traffic, peer restart, and partial transport failure.

### G6 - performance and resource envelope

- [ ] Correctness-equivalent runtime A/B benchmark.
- [ ] Bounded memory, queue, task, socket, descriptor, and handle results.
- [ ] No performance claim without reproducible raw evidence.

### G7 - documentation and release

- [ ] Conformance ledger, PICS/BIBB drafts, API docs, stubs, changelog, version,
      plans, and support summary agree.
- [ ] Release-readiness review has no unresolved blocker/high finding.
- [ ] Rollback ref and cutover procedure are documented.

## Conflict-resolution checklist

For every conflicted or jointly modified file:

1. Identify the public behavior and tests changed on each side.
2. Select an owner using the architectural decision table above.
3. Start from the owner's version; add the other side as a narrow extension.
4. Run both sides' relevant tests before deleting either implementation.
5. Record the resolution and evidence in the reconciliation ledger.

Automatic `ours`/`theirs` resolution is prohibited for:

- `Cargo.toml`, `Cargo.lock`, or workspace membership;
- client dispatch, request, TSM, segmentation, or discovery modules;
- B/IP, BBMD, SC, or transport trait modules;
- server COV, request admission, lifecycle, or PICS modules;
- Python module exports, exceptions, stubs, or API documentation;
- conformance ledgers and public support statements.

## Explicitly obsolete work

The following must not be reintroduced:

- `33af6ed` and `5c4e3b6` invoke-ID-only cross-subnet TSM fallback behavior;
- the orphaned March streaming-I-Am implementation `62add44` in place of W1;
- March routed-addressing implementation structure where current typed targets
  or upstream canonical peers provide the behavior;
- local BBMD fanout, rate-limit, or admission behavior that is weaker than
  upstream;
- local COV worker ownership that duplicates upstream confirmed-notification
  workers;
- pre-0.11 SC behavior that bypasses upstream UUID, TLS, admission, heartbeat,
  or deadline requirements.

Legacy branches may be archived only after the reconciled `dev` passes G0-G7
and a permanent pre-reconciliation ref has been pushed.

## Risks and controls

| Severity | Risk | Control |
|---|---|---|
| blocker | Silent loss of upstream safety/conformance work during conflict resolution | Upstream-first branch, owner table, dual test manifests, no blanket conflict choices |
| blocker | Wrong-peer transaction completion from legacy routed fallback | Retain upstream canonical peer identity; adversarial correlation tests |
| high | Runtime duplicates upstream endpoint transaction ownership | Runtime calls endpoint/client seams; architecture review before PR 7 |
| high | Custom BVLL policy bypasses upstream fail-closed or resource limits | Denial precedence, bounded callback bridge, upstream BBMD suites mandatory |
| high | Local Python port drops new upstream 0.11 APIs | Generated union-surface report and clean-wheel contract gate |
| high | COV port replaces newer quota/worker lifecycle code | Upstream COV engine owns scheduling; local work limited to traits, metadata, projection, and fixtures |
| medium | Container timing/network behavior produces flaky evidence | Run deterministic unit/installed tests first; record environment and retry only whole clean runs |
| medium | Historical performance evidence no longer describes the code | Re-run from one immutable reconciled source archive |
| medium | Long stacked branch becomes difficult to review or roll back | Small PRs, dependency table, per-tranche tags, and permanent old-dev ref |

## Evidence commands

Record exact versions and results for the applicable commands; do not replace
failures with shortened or weakened invocations.

```sh
git rev-parse dev upstream/dev
git merge-base dev upstream/dev
git rev-list --left-right --count dev...upstream/dev
git cherry -v upstream/dev dev
git diff --shortstat 6b9ac4f..dev
git diff --shortstat 6b9ac4f..upstream/dev
git diff --name-status 6b9ac4f..dev
git diff --name-status 6b9ac4f..upstream/dev
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --exclude rusty-bacnet --locked --no-fail-fast
cargo check -p rusty-bacnet --tests --locked
```

Container, wheel, feature-matrix, benchmark, and packaging commands must be
taken from the reconciled versions of the existing test READMEs and evidence
scripts so their source commit and environment are explicit.

## Progress log

- 2026-09-11: Refreshed `upstream/dev` from `6b9ac4f` to `a62821b`. Recorded
  20 local-only versus 352 upstream-only commits, zero patch-equivalent local
  commits, 89 overlapping changed paths, and 55 textual trial-merge conflicts.
- 2026-09-11: Classified W1-W13 as not wholly superseded. Selected upstream
  ownership for endpoint/TSM/segmentation, BBMD safety, COV scheduling, and SC;
  retained local ownership for the aggregate runtime and differentiated Python,
  topology, control, observation, and cross-stack surfaces.
- 2026-09-11: Confirmed that `bacnet-endpoint-core` owns canonical peer and
  transaction coordination but its production adapters remain limited to a
  direct, unsegmented ReadProperty seam. The aggregate runtime therefore
  composes the full upstream `BACnetClient` per attachment and does not add a
  parallel transaction coordinator.
