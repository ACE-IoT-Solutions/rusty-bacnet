# ace-ng-edge Python API parity plan

Status: active
Created: 2026-07-13
Baseline: `65fd871c14e29428e1840d1cddd10f42c0bf4002` (`dev`, rusty-bacnet 0.10.1 workspace)
Source assessment: `../ace-ng-edge/docs/plans/bacpypes3-to-rusty-bacnet-migration-guide.md`

## Goal

Provide a stable, installed-wheel Python API that lets ace-ng-edge reproduce its
current BACnet discovery, explicit routed access, topology, BBMD/FDT inspection,
scan enrichment, and COV workflows without depending on transient discovery
state or losing BACnet address information.

This plan covers rusty-bacnet changes and the evidence needed before the sibling
ace-ng-edge repository can enable a `RustyBacnetStack` adapter. It does not
authorize a production migration or broader public support claims.

## Working rules

- Keep each PR to one API boundary, BACnet layer, or testable state-machine
  family.
- Add tests and conformance evidence with every implementation PR.
- Do not add new CI workflows unless explicitly requested.
- Do not broaden README or support claims until the corresponding acceptance
  rows below are complete.
- Preserve numeric proprietary object/property identifiers and raw address
  bytes.
- Treat `router + DNET + DADR` as persisted input, not data that may be inferred
  only from an in-memory discovery table.
- Exercise the installed Python package for Python acceptance, not only Rust
  internals.

## Definition of migration-ready

All P0 milestones are complete, installed Python tests pass, and an external
parity harness demonstrates:

- direct and routed discovery without address or port loss;
- explicit routed RP/RPM/WP after restart without preparatory Who-Is;
- scoped and unscoped router discovery with complete network claims;
- Read-BDT and Read-FDT with typed results and isolated socket cleanup;
- large segmented object-list reads and indexed fallback;
- direct and routed confirmed/unconfirmed COV with renewal and cancellation;
- stable error normalization at the ace-ng-edge adapter boundary;
- semantically equivalent device and topology snapshots across both stacks.

## Milestone 0 — baseline and contract fixtures (P0)

Purpose: make later API work measurable before changing behavior.

- [x] Add a Python test package under `crates/rusty-bacnet/tests/` that runs
      against an installed extension.
- [x] Assert required runtime exports and core signatures agree with
      `rusty_bacnet.pyi`.
- [x] Add installed-wheel direct RP, RPM, and priority WP loopback coverage.
- [x] Add installed-wheel confirmed COV subscribe, delivery, and unsubscribe
      loopback coverage.
- [x] Add deterministic installed-wheel Who-Is/I-Am discovery response coverage
      using distinct same-port network endpoints. The single-host ephemeral-port
      smoke test exercises the request path but cannot reliably receive the
      server's broadcast I-Am. The Podman Compose fixture verifies the device
      instance, direct source, IPv4 address, and UDP port bytes.
- [x] Add byte-exact fixtures for default/non-default B/IP ports, one-byte DADR,
      multi-byte DADR, and direct versus routed sources.
- [x] Add installed-wheel typed timeout and BACnet Error fixtures with a valid
      follow-up request proving client recovery.
- [x] Add installed-wheel Reject and Abort fixtures with structured reason codes
      and valid follow-up requests proving client recovery.
- [x] Add a malformed APDU response fixture proving decode failure is isolated,
      the request times out predictably, and the client remains usable.
- [x] Add an installed-wheel transport bind failure fixture proving the error is
      typed and a subsequent clean client can start and complete a valid request.
- [x] Add BVLC management error fixtures with the Milestone 3 Python BDT/FDT
      management API.
- [x] Document the local build/install/test command without adding new CI.

Acceptance evidence:

- A clean virtual environment can install the locally built wheel and run the
  Python suite.
- A subsequent request succeeds after each injected error, proving worker and
  dispatcher survival.

## Milestone 1 — typed target and explicit routed property operations (P0)

Purpose: make persisted routed addresses first-class and independent of Who-Is.

- [x] Add frozen Python `DirectTarget` and `RoutedTarget` records.
- [x] Validate router B/IP MAC length, DNET range, DADR length, and unsupported
      broadcast forms at construction.
- [x] Add symmetric Rust core `read_property_multiple_routed`,
      `write_property_routed`, and `write_property_multiple_routed` operations.
- [x] Bind explicit routed RP, RPM, WP, and WPM using the typed target.
- [x] Retain device-instance auto-routing as a convenience API.
- [x] Update `rusty_bacnet.pyi` and Python API documentation.
- [x] Add direct/routed segmented-response tests using the same target model.

Acceptance evidence:

- A fresh process with an empty device table performs RP, RPM, and priority WP
  through a persisted router/DNET/DADR target.
- Tests cover a non-default router UDP port and a multi-byte DADR.
- No public Python method exposes raw NPDU construction.

## Milestone 2 — client router discovery (P0)

Purpose: expose topology discovery and route claims through one reusable client
implementation rather than a CLI-only path.

- [x] Add a client/network-layer Who-Is-Router-To-Network initiator.
- [x] Correlate and collect I-Am-Router-To-Network responses for a caller-defined
      observation window.
- [x] Support unscoped and DNET-scoped queries.
- [x] Preserve the complete responder MAC, including non-default B/IP port.
- [x] Merge repeated responses per source while retaining newly advertised
      networks.
- [x] Represent success, timeout with partial results, and protocol/transport
      failure distinctly.
- [x] Expose an immutable Python `RouterInfo` result and optional route snapshot.
- [x] Replace the CLI TODO with the same client API.

Acceptance evidence:

- Tests include two routers, duplicate replies, distinct network sets, scoped
  silence for an unknown network, and partial timeout.
- Python results contain exact source bytes and sorted unique network numbers.
- A learned result supplies enough data to construct a `RoutedTarget`.

## Milestone 3 — Python Read-BDT and Read-FDT (P0)

Purpose: expose Annex J topology inspection without leaking the concrete Rust
transport type through Python.

- [x] Choose and document a B/IP-only management facade or ephemeral helper.
- [x] Add frozen Python `BdtEntry` and `FdtEntry` records.
- [x] Expose async `read_bdt` and `read_fdt` with explicit timeout.
- [x] Preserve IP, port, broadcast mask, TTL, and seconds remaining exactly.
- [x] Reject use with SC/IPv6 using a stable typed error.
- [x] Support a fresh ephemeral socket per management probe.
- [x] Verify correlation by peer and expected BVLC function.
- [x] Verify every success, timeout, malformed reply, and cancellation path
      releases the socket and pending correlation state.

Acceptance evidence:

- Installed Python tests pass against rusty-bacnet BBMD fixtures.
- Cross-stack tests pass against a bacpypes3 BBMD fixture.
- Repeated probes leave the UDP port re-bindable and observe FDT expiry changes.

## Milestone 4 — routed and managed COV (P0)

Purpose: preserve ace-ng-edge cache, renewal, and polling-suppression behavior.

- [x] Accept `DirectTarget` and `RoutedTarget` for SubscribeCOV and cancellation.
- [x] Expose a managed subscription handle with explicit async close/cancel.
- [x] Bind managed renewal state and failure reporting.
- [x] Surface notification channel lag/overflow instead of logging and silently
      skipping events.
- [x] Test confirmed and unconfirmed delivery, initial notification, renewal,
      cancellation, expiry, and shutdown races.
- [x] Test routed subscriptions after restart without discovery priming.

Acceptance evidence:

- Notification loss is observable by the Python caller.
- Confirmed notification handling does not duplicate delivery.
- All managed tasks and sockets terminate after cancellation and client stop.

## Milestone 5 — scan metadata and value contract (P0, shared ownership)

Purpose: prevent point/property coverage regressions when bacpypes3 reflection is
removed.

Rusty-bacnet scope:

- [x] Guarantee lossless numeric object/property identifiers in Python.
- [x] Complete Python conversion tests for Null, enum, bit string, object ID,
      octet string, array/list, constructed values, and unknown raw values.
- [x] Expose per-property RPM errors without losing response order or array
      indexes.

ace-ng-edge scope:

- [x] Create a versioned, checked-in property catalog keyed by numeric object
      type, with optionality, value category, scan priority, and COV relevance.
- [x] Define the unknown/proprietary object fallback explicitly.
- [x] Diff enriched scan output against bacpypes3 fixtures.

Acceptance evidence:

- Enriched property coverage is no lower than the accepted bacpypes3 baseline.
- Proprietary identifiers and values survive scan serialization unchanged.

## Milestone 6 — multi-attachment aggregate (P0, ace-ng-edge owned initially)

Purpose: reproduce the live NetworkPort lifecycle without first building a new
native multi-port Python application.

- [x] Assign a stable attachment ID to each `BacnetNetworkConfig`.
- [x] Own one rusty-bacnet client per attachment behind the adapter.
- [x] Define broadcast fan-out and duplicate I-Am selection.
- [x] Preserve ingress attachment on discovered and routed devices.
- [x] Define route and COV subscription ownership.
- [x] Reconcile add, update, remove, rollback, and UDP bind conflicts.
- [x] Aggregate notifications without duplicates.
- [x] Prove removed attachments release tasks, sockets, device state, and routes.

Acceptance evidence:

- Repeated add/update/remove cycles return resources to baseline.
- Discovery and routed calls select the correct source attachment.
- Distinct B/IP networks are never implicitly treated as BBMD peers.

## Milestone 7 — cross-stack parity and release evidence (P1)

- [ ] Run bacpypes3 client versus bacpypes3 fixtures.
- [ ] Run rusty-bacnet client versus bacpypes3 fixtures.
- [ ] Run bacpypes3 client versus rusty-bacnet fixtures.
- [ ] Run rusty-bacnet client versus rusty-bacnet fixtures.
- [ ] Cover routers, BBMD peers, live FDT expiry, non-default ports, routed
      MS/TP-style DADR, duplicate network claims, and a large segmented object
      list.
- [ ] Compare normalized device inventory, topology snapshots, scan properties,
      error categories, and COV delivery.
- [ ] Exercise installed wheels for every supported Python/platform artifact.
- [ ] Rehearse configuration rollback in ace-ng-edge.

Acceptance evidence:

- No unexplained topology delta or loss of routed-device access.
- Read-result agreement and performance meet the gates in the migration guide.
- Persisted models require no migration when switching stacks or rolling back.

## PR queue

| Order | Proposed PR | Repository | Depends on | Status |
|---:|---|---|---|---|
| 1 | Installed Python contract harness | rusty-bacnet | none | complete locally |
| 2 | Typed direct/routed targets | rusty-bacnet | PR 1 | complete locally |
| 3 | Explicit routed RP/RPM/WP/WPM | rusty-bacnet | PR 2 | complete locally |
| 4 | Router discovery collector and binding | rusty-bacnet | PR 1 | complete locally |
| 5 | Read-BDT/Read-FDT Python management API | rusty-bacnet | PR 1 | complete locally |
| 6 | Managed routed COV | rusty-bacnet | PR 2 | complete locally |
| 7 | Value/error parity completion | rusty-bacnet | PR 1 | complete locally |
| 8 | Property catalog and neutral adapter | ace-ng-edge | PRs 2-7 | complete locally |
| 9 | Multi-attachment aggregate | ace-ng-edge | PRs 2-7 | complete locally |
| 10 | Four-way parity harness and rollout evidence | both | PRs 1-9 | not started |

## Progress log

- 2026-07-13: Confirmed local `dev`, `upstream/dev`, and fork `origin/dev` all
  resolve to `65fd871c14e29428e1840d1cddd10f42c0bf4002`.
- 2026-07-13: Initial API, architecture, edge-contract, and validation review
  completed. Rust core router/BVLL/TSM/segmentation/COV focused suites passed;
  installed-Python parity coverage remains the first blocker.
- 2026-07-13: Added the dependency-free installed-package contract harness and
  clean wheel build/install instructions. Runtime network loopback coverage is
  still open.
- 2026-07-14: Added installed-wheel B/IP loopback tests for RP, RPM, priority
  WP, and confirmed COV. The discovery request path is covered, but deterministic
  I-Am receipt remains open for a same-port multi-endpoint network fixture.
- 2026-07-14: Added a two-container, same-port installed-wheel Who-Is/I-Am
  fixture with byte-exact direct B/IP MAC assertions. The fixture passes with
  Podman 6.0.0 and podman-compose 1.6.0. Added passing typed protocol-error and
  timeout recovery tests to the local installed-wheel suite.
- 2026-07-14: Added raw UDP fault responders for correlated Reject, Abort, and
  malformed APDU responses. Installed Python exposes structured Reject/Abort
  reasons, malformed replies degrade to timeout, and the client succeeds on a
  valid request after every fault.
- 2026-07-14: Added a deterministic occupied-port transport startup failure and
  verified a subsequent clean client can bind and complete a valid read.
- 2026-07-14: Added immutable `DirectTarget` and `RoutedTarget` Python records.
  A clean CPython 3.13 wheel passed all 13 installed-package tests, including
  byte-exact non-default B/IP ports, multi-byte DADR preservation, immutability,
  and constructor rejection of unsupported broadcast and invalid address forms;
  `cargo check -p rusty-bacnet --offline` also passed.
- 2026-07-14: Added symmetric routed RPM/WP/WPM Rust operations and dispatched
  Python RP/RPM/WP/WPM through typed direct or routed targets while retaining
  string and device-table convenience APIs. The 64-test `bacnet-client` suite
  passed, and a clean installed wheel passed 14 tests including fresh-client
  routed RP, RPM, priority WP, and WPM through a non-default router UDP port
  with a three-byte DADR. Segmented target-model coverage remains open.
- 2026-07-14: Added direct and routed installed-wheel segmented ComplexACK
  coverage using the typed target model. The routed case exposed and fixed a
  missing routed NPDU on client SegmentACKs. All 64 `bacnet-client` tests and
  all 15 clean installed-wheel tests pass; Milestone 1 is complete locally.
- 2026-07-14: Extended routed segmented-session control replies to preserve the
  destination network/address for both SegmentACK and timeout Abort paths. The
  64-test client suite remained green, and the Podman discovery fixture rebuilt
  and passed with the Linux CPython 3.11 wheel (exit 0) before Milestone 2 work.
- 2026-07-14: Completed Milestone 2 router discovery across the network layer,
  reusable client collector, immutable Python `RouterInfo`/snapshot API, and
  CLI. Fixed unscoped Who-Is-Router encoding to omit the optional DNET instead
  of emitting network zero. The 69-test network suite and 67-test client suite
  pass; a clean CPython 3.13 installed wheel passes all 15 tests; and the Podman
  Linux CPython 3.11 fixture passes with two routers, duplicate merged claims,
  exact B/IP source/port bytes, scoped discovery, and unknown-network silence.
  A cancellation-focused Rust test verifies partial results remain available
  through `router_snapshot()` while protocol/transport failures remain errors.
- 2026-07-14: Completed Milestone 3 with a B/IP-only ephemeral management
  facade, frozen lossless `BdtEntry`/`FdtEntry` records, explicit timeouts, and
  typed `BacnetBvlcError` result codes. All 309 `bacnet-transport` tests pass. A
  clean CPython 3.13 installed wheel passes all 19 tests; focused management
  fixtures additionally rebind every ephemeral source socket after success,
  timeout, malformed ACK, BVLC NAK, and cancellation. The Podman Linux wheel
  fixture passes against bacpypes3 0.0.102 with exact non-default BDT/FDT ports,
  masks, TTL, and a decreasing live FDT remaining time. Existing transport
  correlation tests verify exact peer and expected BVLC response function.
- 2026-07-14: Started Milestone 4 by adding symmetric explicit routed
  SubscribeCOV/cancellation core methods and accepting the shared
  `DirectTarget`/`RoutedTarget` Python model. Focused Rust tests prove both
  requests use the persisted router/DNET/DADR with an empty device table, and
  an installed-wheel test passes a confirmed routed subscription,
  notification, and cancellation without discovery priming. COV iterator lag
  now raises `BacnetNotificationLagError(skipped=...)` instead of logging and
  silently discarding the loss; managed-handle binding and its full lifecycle
  matrix remain open.
- 2026-07-14: Completed Milestone 4 with a managed finite COV handle, explicit
  idempotent close/cancel, durable renewal state and failure events, direct and
  explicit routed renewal, and observable notification/event lag. Core tests
  cover observed expiry, renewal, timeout failure, and cancellation during an
  in-flight renewal; all 67 client unit tests and 3 integrations pass. A clean
  CPython 3.13 wheel passes all 24 installed-package tests, including confirmed
  and unconfirmed initial delivery, overflow reporting, renewal failure after
  TSM retries, routed renewal after a client restart without discovery priming,
  and client-stop task/socket release. The Podman Linux wheel, router discovery,
  and bacpypes3 BBMD fixture completed with client exit 0.
- 2026-07-14: Completed the rusty-bacnet portion of Milestone 5. Installed-wheel
  tests preserve proprietary numeric object/property identifiers, cover Null,
  enumerated, bit-string, proprietary object-ID, octet-string, and nested list
  conversion, and return undecodable application-tag values as byte-exact raw
  bytes. RPM loopback coverage proves mixed success/error results retain request
  order, proprietary property IDs, array indexes, and typed error class/code.
  The clean CPython 3.13 wheel passes all 27 installed-package tests.
- 2026-07-14: Completed the ace-ng-edge portion of Milestone 5 with a checked-in
  v1 catalog generated from bacpypes3 0.0.102. It covers all 63 standard object
  types and 1,771 readable property rows keyed by numeric identifiers, including
  required/optional status, value category, scan priority, and COV relevance.
  Runtime enrichment now selects properties from the catalog without object
  class reflection. Unknown names and numeric proprietary types explicitly try
  object-name, description, and present-value; proprietary property identifiers
  remain numeric strings and raw values serialize losslessly as `raw_hex`.
  Catalog regeneration and all-object-type reflection parity tests pass; 104
  focused ace-ng-edge BACnet tests pass, and the built wheel contains the JSON
  catalog.
- 2026-07-14: Completed Milestone 6 with an ace-ng-edge aggregate that owns one
  rusty-bacnet client, notification pump, route state, and managed-COV handle
  set per durable `network_attachment_id`. Reconciliation validates attachment,
  operator, and socket conflicts; preserves metadata-only clients; replaces
  changed binds; closes removed resources; and restores the prior attachment
  after bind or subscription-cleanup failure without disturbing peers.
  Discovery fans out in reconcile order, records immutable ingress provenance,
  applies deterministic first-attachment duplicate precedence, and isolates
  per-attachment failures. Explicit routed, property, BVLL, and COV calls cannot
  cross attachment boundaries. A Podman fixture with two isolated bridges
  exposed and fixed cross-interface I-Am delivery by binding wildcard B/IP
  sockets to their owning OS interfaces. The final installed-wheel fixture
  proves duplicate precedence, distinct-network provenance, removal/re-add, and
  per-interface UDP release. Evidence: 309 transport tests, 67 client tests, 27
  clean CPython 3.13 rusty-bacnet wheel tests, 187 focused edge tests, 11
  aggregate lifecycle tests, a clean installed edge/Rusty wheel lifecycle test,
  and the Podman aggregate fixture all pass.
- 2026-07-14: Closed the remaining Milestone 0 byte contract with clean-wheel
  assertions for default and non-default B/IP port bytes, one-byte DADR, and a
  leading-zero multi-byte DADR across direct and routed targets. All P0
  milestones are complete locally; cross-stack rollout evidence remains P1.
