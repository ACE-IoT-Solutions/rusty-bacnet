# SC-to-IP router work plan

Status: upstream 0.14.0 merged on `feature/upstream-0.14-migration` (`ae3cec68`,
2026-10-23) on top of the 0.13 migration line and `main`; fork delta is now 51
files over upstream `dev`. I1, I3, I6 overtaken by the merge; I2, I4, I5, I7, I8
open. R10 open.
Created: 2026-09-11
Target line: `main` at `9eed26a` (workspace version 0.11.0, reconciled onto
`upstream/dev` `a62821b`)
Kickoff pin (2026-09-11): `main` `9eed26a`. The proposal branch was last
reconciled to `upstream/dev` `0376fa3` (2026-09-12); the latest fetched
`upstream/dev` is `3364a8d` (2026-09-19), another 119 commits later. Those
commits include further SC reconnect, recovery, port, and NPDU-admission work.
The C4 audit below confirms that R1-R6's health, topology, unbounded-retry,
runtime, and Python surfaces remain unique, while a replacement proposal must
selectively adopt newer upstream safety and admission work. The old proposal
is retained as evidence and is not submission ready.
Feedback baseline: fork `dev` at `bf6922d` (workspace version 0.10.1)
Related plans: `docs/plans/upstream-reconciliation-workplan.md`,
`docs/plans/bacpypes3-feature-parity-workplan.md`

Implementation note: the router team's referenced section 5.1 is not present
in this repository. The names and keyword mapping in this plan are therefore
provisional pending external owner confirmation; implementation proceeds with
`RouterBipPort`, `RouterScPort`, and `RouterVirtualPort` as written here.

## Goal

Let a downstream Python application build a long-lived BACnet/SC-to-BACnet/IP
router from an installed `ace-rusty-bacnet` wheel, with typed per-port
configuration, unbounded SC reconnection, per-port health and routing-table
visibility, correct SC identity, and standards-conformant handling of
oversize NPDUs. Fork-only pre-1.0 Python compatibility is intentionally not
an upstream constraint; portable protocol changes remain independently
reviewable.

## Blocking implementation status

The source contains the R1 typed Python port surface, R3 unbounded policy and
deterministic recovery tests, R4 Rust/Python health and route snapshots, and
the R5 explicit runtime/router SC device identity.
R2 has mixed Rust unit and mutual-TLS integration coverage plus installed-wheel
and W14 Linux arm64 execution evidence. Local focused, workspace, installed
wheel, W5, and W14 gates are green. CI execution remains a delivery gate, not
an implementation gap in the blocking scope.

The implementation now includes dispatch-time oversize-NPDU rejection and
counters, configurable router-capable SC receive limits (R7), periodic and recovery-triggered
I-Am-Router-To-Network announcements (R8), and fork wheel publication (R9).
Multi-arch images (R10) remain open. At an SC-to-B/IP boundary, SC-only Data Options are
ignored by B/IP while the NPDU is forwarded; this boundary behavior is not an
oversize or general Annex AB conformance claim.

The router team's requests are tracked here as R1-R10:

| ID | Tier | Request |
|---|---|---|
| R1 | Blocking | Expose SC as a router port in the Python `BACnetRouter` |
| R2 | Blocking | Mixed-transport (B/IP to SC) router integration test |
| R3 | Blocking | Unbounded SC reconnect, or router-side restart of a dead port |
| R4 | Blocking | Per-port health and a routing-table snapshot from Python |
| R5 | Important | Explicit SC device UUID on every SC construction path |
| R6 | Important | `transport_kind()` and `topology_id()` on `ScTransport` |
| R7 | Important | Oversize NPDU handling on the SC leg |
| R8 | Nice | Periodic I-Am-Router-To-Network re-announce |
| R9 | Nice | Tag and publish wheels from the fork |
| R10 | Nice | Multi-arch container images |

## Baseline assessment

### Re-anchored references

The feedback cites the fork's `dev` branch. `main` has since been rebuilt on
upstream 0.11.0, so every location moved. Verified positions on `main`:

| Feedback cite (`dev` 0.10.1) | `main` location | Notes |
|---|---|---|
| `bacnet-transport/src/any.rs:28-46` | `crates/bacnet-transport/src/any.rs:28-46` | `AnyTransport::Sc(Box<ScTransport<TlsWebSocket>>)` at :41 behind `sc-tls` |
| `rusty-bacnet/src/router.rs:144-151, :297-311` | `crates/rusty-bacnet/src/router.rs:142-158, :296-311` | Constructor still `(bip_network, virtual_ports, ...)`; `start()` builds one `Bip` port plus `Virtual` ports |
| `router/tests/data_attributes.rs:141-165` | `crates/bacnet-network/src/router/tests/data_attributes.rs:141` | Only SC-to-SC over `LoopbackWebSocket` |
| `sc/mod.rs:678-687` | `crates/bacnet-transport/src/sc/mod.rs:633-642` and `sc/recovery.rs:62-155` | Recovery returns `None` after `max_retries`; receive task hits `break 'transport` |
| `bacnet-runtime/.../configuration.rs` (`max_retries == 0`) | `crates/bacnet-runtime/src/transport/configuration.rs:195-203` | Still rejects zero |
| `configuration.rs:248` (UUID from `attachment_id`) | `configuration.rs:321` | `.with_device_uuid(*config.id.as_bytes())` |
| `router/runtime.rs:584` (`table()`) | `crates/bacnet-network/src/router/mod.rs:602` | Rust-only |
| `sc/mod.rs:180` (`connection_state_changes()`) | `sc/mod.rs:228` | Rust-only `watch::Receiver<ScConnectionState>` |
| `router/runtime.rs:36-44` (duplicate-segment guard) | absent | See regression below |
| `sc/connection.rs:61-64` | `sc/connection.rs:61-64` | Local Max-BVLC now `sc_limits::DEFAULT_MAX_BVLC_LENGTH` (5705); local Max-NPDU and both hub values still 1476 |
| `sc/send.rs:47` | `sc/send.rs:47-53` | `NPDU length exceeds peer Max-NPDU-Length` |
| `router/runtime.rs:258-293` (announce once) | `router/mod.rs:320-356` | Still once at start |
| `ci.yml:468-473` | `.github/workflows/ci.yml:475-480` | Multi-arch TODO |

### Regression introduced by the reconciliation

`dev` carried `TransportPort::transport_kind()` and `TransportPort::topology_id()`
with default implementations (`dev:crates/bacnet-transport/src/port.rs:92-100`),
per-transport overrides for B/IP and virtual networks, an `AnyTransport`
pass-through, and a duplicate-topology guard in the router
(`dev:crates/bacnet-network/src/router/runtime.rs:36-44`). None of that
survived onto `main`. `crates/bacnet-network/src/router/mod.rs:248` again
reports `std::any::type_name::<T>()` as `transport_kind`, and the reconciliation
ledger records Phase 8 as "ported as a bounded simulation adapter" without a
row for transport identity. R6 is therefore a restore-and-extend, not new work.

### What `main` already satisfies

- `BACnetClient`, `BACnetServer`, `ScHub`, `RuntimeScAttachment`, and the
  router SC port require caller-provisioned nonzero 16-byte device identities.
- The Rust router already learns routes, ages them, answers
  Who-Is-Router-To-Network, and emits Reject-Message-To-Network for unknown or
  busy networks (`router/forwarding.rs:187`, `router/control_messages.rs`).
- `ScReconnectConfig` already treats `max_retries == 0` as "no reconnect
  retries" (`sc/reconnect.rs:11-13`), so zero cannot be reused to mean
  unbounded.
- `RouterTable` exposes everything a snapshot needs
  (`crates/bacnet-network/src/router_table.rs:219-280`).
- `ActiveHub::{Primary, Failover}` exists but is private to `sc::failover`.
- `RejectMessageReason::MESSAGE_TOO_LONG` (reason 4) exists in
  `crates/bacnet-types/src/enums/network.rs:49`.
- The Python test suite runs against an installed wheel. On the 0.12 line the
  router lifecycle contract is covered by `test_router_ports.py`; the removed
  fork-only `test_virtual_router.py` is historical evidence only.
- The conformance ledger row `BACNET-6-ROUTER-MESSAGES` is
  `implementation-present-needs-conformance-tests`
  (`docs/conformance/standard-135-2020-ledger.md:371`).

## Working rules

- The earlier additive-only rule is superseded for the pre-1.0 0.12
  reconciliation. The fork-only
  `BACnetRouter(bip_network, virtual_ports, ...)` constructor, virtual router
  port, and old forwarding counters are intentionally removed; callers use
  `BACnetRouter.from_ports()` and typed B/IP/SC ports.
- One PR per layer, in dependency order, each with its tests. A PR that only
  touches `bacnet-transport` or `bacnet-network` must be proposable to
  upstream as-is; fork-specific packaging stays in its own PR.
- Before opening each PR, fetch `upstream/dev` and check whether upstream has
  since landed an equivalent. Prefer adopting upstream's shape and record the
  upstream commit in the PR description.
- Do not weaken upstream SC safety: retry eligibility after a NAK, TLS 1.3 and
  mutual-TLS requirements, non-zero UUID and VMAC validation, and
  connection-state ownership stay exactly as they are.
- Prefer configuration over behavior changes. Any new periodic traffic
  (re-announce) and any raised size limits default to today's behavior unless
  the standard fixes the value.
- Python acceptance runs against an installed wheel built with `maturin`.
  Any finding that changes `crates/rusty-bacnet` or Python-visible behavior is
  closed only after the full Python suite passes in a fresh isolated
  environment against that wheel with checkout imports disabled.
- Update `rusty_bacnet.pyi`, `docs/python-api.md`, `docs/rust-api.md`, and
  `CHANGELOG.md` in the same PR as the API.
- Ledger status changes require a test that actually exercises the clause.

## Architectural decisions

| Boundary | Decision | Rationale |
|---|---|---|
| Transport identity (R6) | Restore `transport_kind()` and `topology_id()` as defaulted `TransportPort` methods; return `"bip"`, `"sc"`, `"mstp"`, `"bip6"`, `"ethernet"`, `"loopback"`, `"virtual"`; SC topology is `primary_hub_url` normalised plus failover URL | Matches the `dev` design; default bodies keep upstream implementors compiling |
| Transport health (R4) | New defaulted `TransportPort::health()` returning a `TransportHealth` snapshot, plus optional `health_changes()` watch; `ScTransport` implements both with state, active hub, last error, attempt counter, and connected-since | Router stays transport-agnostic; B/IP returns a static `Up` |
| Unbounded retry (R3) | Add `retry_forever: bool` to `ScReconnectConfig` (default `false`), keep `max_retries` semantics; when forever, alternate primary and failover each cycle, keep exponential backoff capped at `max_delay_ms`, still stop on `connect_retry_allowed == false` | Preserves the standard's "do not retry" NAK outcomes; smallest surface change |
| Dead-port visibility (R3/R4) | A port whose receive task exits reports `state = "failed"` with the terminal reason in health; router-side port restart is deferred, see Open decisions | Health first makes restart observable before automating it |
| Python ports (R1) | Typed frozen classes `RouterBipPort`, `RouterScPort`, `RouterVirtualPort`; `BACnetRouter.from_ports(ports)` static constructor; legacy `__init__` delegates internally | Names to be reconciled with section 5.1 of the router team's plan before the PR merges; the R8 interval keyword stays absent until implemented |
| SC dial path | Extract one `sc-tls` helper that turns an SC port config into a started-ready `ScTransport<TlsWebSocket>` and reuse it from runtime, client, server, and router | Today runtime, `bacnet-server/sc_builder.rs`, and Python lifecycles each dial separately |
| SC UUID (R5) | `RuntimeScAttachment` has required keyword-only `device_uuid: bytes`; `RouterScPort` requires it; the `attachment_id` derivation is removed rather than kept as a fallback | Operators need a persisted identity; a silent fallback would hide misconfiguration |
| Oversize NPDU (R7) | Two parts: (a) size the SC leg to the standard, making local Max-NPDU and hub-side limits configurable and defaulting to the Annex AB values already cited in `sc_limits.rs`; (b) a defaulted `TransportPort::max_npdu_length()` hint so the router rejects in the dispatch path with reason 4 and a new `oversize_drops` counter | Rejecting at dispatch time is where SNET/SADR is still known; the sender task cannot reply |
| Routing table snapshot (R4) | `BACnetRouter::routing_table()` returns `Vec<RouteSnapshot>` cloned under the lock; Python `routing_table()` returns frozen `RouterRouteEntry` | Never hand the `Arc<Mutex<RouterTable>>` across the FFI boundary |
| Re-announce (R8) | `RouterConfig { announce_interval: Option<Duration> }` via `BACnetRouter::start_with_config`; also re-announce on any port health transition to `Up` | Reconnected SC hubs need the announcement more than a timer does |
| Publishing (R9) | Distribution name `ace-rusty-bacnet`, module name `rusty_bacnet` unchanged; wheels built from fork tags only, never from upstream tags or under upstream's PyPI name | Decided 2026-09-11 |
| Containers (R10) | Extend `ci.yml` to build `linux/amd64` and `linux/arm64` images for a router image based on the wheel; Podman locally | Upstream's TODO block already names the target |

## Phase 0 - freeze baseline and reconcile naming

Purpose: pin what the plan targets and remove the naming ambiguity before code.

- [x] Record `main` and `upstream/dev` SHAs in this plan's header at kickoff.
- [x] Record that section 5.1 of the router team's plan is unavailable locally;
      map the provisional `RouterBipPort` / `RouterScPort` kwargs onto the
      existing runtime naming and record that the names require team review.
- [x] Run the full workspace suite and the installed-wheel Python suite to
      capture a green baseline.
- [x] Add ledger evidence for the blocking phases under
      `docs/plans/evidence/sc-ip-router/`. A general kickoff/source inventory
      and final local gate results are recorded.

Acceptance: header pinned, kwargs table agreed, baseline green.

## Phase 1 - transport identity and health (R6, R4 foundation)

Crates: `bacnet-transport`.

- [x] Restore `transport_kind()` and `topology_id()` on `TransportPort` with
      default bodies (port from `dev:crates/bacnet-transport/src/port.rs:92-100`).
- [x] Implement both for B/IP, B/IPv6, MS/TP, Ethernet, loopback, virtual
      network, and SC; SC returns `"sc"` and a topology id derived from the
      normalised primary hub URL plus failover URL.
- [x] Add `AnyTransport` pass-through for both (port from `dev:any.rs:48-75`).
- [x] Add `TransportHealth { state, detail, active_hub, last_error, attempt,
      since }` and defaulted `TransportPort::health()` and
      `TransportPort::health_changes() -> Option<watch::Receiver<TransportHealth>>`.
- [x] Make `ActiveHub` public and track `last_error`, `attempt`, and
      `connected_since` inside `ScTransport`; publish through the new watch on
      every state transition, reconnect attempt, and terminal exit.
- [x] Add `max_npdu_length()` defaulted hint; SC returns the effective
      encoded-NPDU ceiling negotiated from the Hub's Max-NPDU and Max-BVLC
      values after Connect-Accept.

Tests: unit tests per transport for kind and topology; SC health sequence over
`LoopbackWebSocket` covering connect, disconnect, reconnect, failover, and
retry exhaustion.

Acceptance: `cargo test -p bacnet-transport --all-features` green; no
signature change to existing trait methods.

## Phase 2 - unbounded SC reconnect (R3)

Crates: `bacnet-transport`, `bacnet-runtime`.

- [x] Add `retry_forever: bool` to `ScReconnectConfig` and a
      `ScReconnectConfig::unbounded(initial_delay_ms, max_delay_ms)` helper;
      update `validate()` and its tests.
- [x] Rework `Recovery::reconnect` (`sc/recovery.rs:62-155`) into a loop that
      honours `retry_forever`, alternates primary and failover per cycle when
      unbounded, keeps capped exponential backoff, and still exits on
      `connect_retry_allowed == false`.
- [x] On terminal exit (`sc/mod.rs:641`) publish `TransportHealth::Failed`
      with the reason before breaking.
- [x] Runtime: accept `reconnect_forever` on the SC attachment config; keep
      rejecting `max_retries == 0` unless `reconnect_forever` is set.
- [x] Update in-repo struct literals (`configuration.rs:325-329`) and the
      existing redial and reconnect validation tests.

Source-compatibility note: adding `retry_forever` to the public Rust struct is
not source-compatible for complete external `ScReconnectConfig { ... }`
literals. Callers must add `retry_forever: false`, use `..Default::default()`,
or use `ScReconnectConfig::unbounded(...)`.

Tests: deterministic `tokio::time::pause` tests proving an unbounded transport
recovers after N failures greater than any bounded budget, that failover is
tried every cycle, and that a retry-forbidden NAK still terminates.

Acceptance: existing `redial_tests.rs`, `reconnect_validation_tests.rs`, and
`primary_restore_tests.rs` stay green; new tests green.

## Phase 3 - router runtime extensions (R4, R7, R8 core)

Crates: `bacnet-network`.

- [x] Restore the duplicate-topology guard in `BACnetRouter::start`
      (`router/mod.rs:213-223`) using `transport_kind()` and `topology_id()`,
      and populate `RouterPortCounters::transport_kind` from the trait instead
      of `type_name`.
- [x] Add `RouterPortHealth` and `BACnetRouter::port_health()` built from each
      port's `health()`; subscribe to `health_changes()` where available.
- [x] Add `RouteSnapshot` and `BACnetRouter::routing_table()`.
- [x] Add `RouterConfig { announce_interval: Option<Duration> }` and
      `start_with_config`; `start` delegates with defaults. Re-announce on the
      timer and on any port transition to `Up`.
- [x] Oversize handling in the dispatch path: when the target port's
      `max_npdu_length()` is known and the NPDU exceeds it, send
      Reject-Message-To-Network reason 4 to the originator via the existing
      `send_reject`, increment a new `oversize_drops` counter, and do not
      forward. Keep the sender-task `send_errors` path for anything else.
- [x] The 0.12 router has no `port_counters()` interface to extend; expose the
      additive router-wide `oversize_drops()` counter instead.

Tests: duplicate-topology rejection; health snapshot ordering matches config
order; routing table snapshot after a learned route; re-announce fires on the
timer and on simulated port recovery; oversize NPDU from B/IP toward a
1476-byte SC port produces a reject with the right DNET and reason.

Acceptance: `cargo test -p bacnet-network` green including upstream's
data-option, priority, hop-count, and rejection tests.

## Phase 4 - SC size limits to the standard (R7 completion)

Crates: `bacnet-transport`, `bacnet-server` (hub).

- [x] Add `with_max_npdu_length` and `with_max_bvlc_length` builders on
      `ScTransport`; validate against `sc_limits` and the u16 fields in
      Connect-Request.
- [x] Set the node default Max-NPDU to the Annex AB value already cited in
      `sc_limits.rs` (1497) rather than 1476, and confirm against the
      standard before merging; document the effective APDU budget.
- [x] Make the hub's advertised Max-BVLC and Max-NPDU configurable in the
      Rust hub and expose them on Python `ScHub`.
- [x] Extend the native max-APDU and installed-wheel configuration contracts
      for the new limits and for a B/IP-sized 1476-byte APDU crossing the SC
      leg intact.

Acceptance: a 1476-byte APDU with a full routed NPCI forwards from B/IP to SC
without hitting `send.rs:47`; anything larger is rejected with reason 4.

## Phase 5 - shared SC dial helper and explicit UUID (R5)

Crates: `bacnet-transport`, `bacnet-runtime`, `bacnet-server`, `rusty-bacnet`.

- [x] Introduce a single `sc-tls` helper that takes an SC port config
      (hub URLs, VMAC, UUID, validated TLS policy, heartbeat, reconnect) and
      returns a constructed `ScTransport<TlsWebSocket>` using the existing
      `select_initial_sc_websocket` failover semantics.
- [x] Route `configuration.rs`, `bacnet-server/src/server/sc_builder.rs`, and
      the Python client and server lifecycles through it without changing
      their behavior.
- [x] `RuntimeScAttachment`: add required keyword `device_uuid: bytes` with
      the same 16-byte non-zero validation used by `BACnetClient`; remove the
      `attachment_id` derivation at `configuration.rs:321`.
- [x] Update `test_runtime_reconciled.py`, `sc_runtime_fixture.py`, and any
      container fixture that builds `RuntimeScAttachment`.

Acceptance: every `ScTransport::new(...)` call site outside tests goes through
the helper; the runtime rejects a missing or zero UUID before dialing.

## Phase 6 - Python typed ports and `from_ports` (R1, R4 surface)

Crates: `rusty-bacnet`.

- [x] Add frozen `RouterBipPort` and `RouterScPort`
      classes with validation at construction (network number, VMAC, UUID,
      PEM presence, heartbeat timing, and reconnect ranges).
- [x] Add `BACnetRouter.from_ports(ports)`;
      require at least two ports and at most one B/IP port per interface and
      UDP port. The removed upstream virtual transport and legacy constructor
      are not revived on 0.12.
- [x] Consolidate `start()` SC construction through the Phase 5
      helper. Current SC dial failures already surface as `RuntimeError`
      naming the port index and hub URL.
- [x] Add `port_health() -> list[RouterPortHealth]` and
      `routing_table() -> list[RouterRouteEntry]` as awaitables following the
      `port_counters()` stop-aware pattern. The R7 `oversize_drops()` counter
      is exposed on the router; explicit SC receive-limit configuration is
      completed in Phase 4.
- [x] Update `rusty_bacnet.pyi` and `docs/python-api.md` for the new surface.
- [x] Record installed-wheel API-contract/manifest verification; no static
      `test_api_contract_manifest.py` expectation change was required.

Tests: `test_router_ports.py` covers typed validation, construction, health
and routing-table shapes, stopped snapshots, and restart. The removed
`test_virtual_router.py` is not a 0.12 compatibility gate.

Acceptance: installed wheel exposes the new classes with the agreed kwargs.

## Phase 7 - mixed-transport evidence (R2)

- [x] Rust unit test in `router/tests/`: `BipTransport` on `127.0.0.1:0` plus
      `ScTransport` over `LoopbackWebSocket`; forward unicast both ways and a
      broadcast from B/IP, asserting SNET/SADR insertion and broadcast
      hop-count decrement. SC-only Data Options are not asserted across the
      B/IP boundary because B/IP intentionally ignores them.
- [x] Rust integration test in `bacnet-integration-tests`: B/IP
      `BACnetClient` routed ReadProperty through the router to an SC
      `BACnetServer` behind the Rust hub, using independently generated
      in-memory mutual-TLS credentials.
- [x] Installed-wheel Python test `test_sc_ip_router.py`: source and execution
      cover
      `ScHub` plus SC
      `BACnetServer`, `BACnetRouter.from_ports([RouterBipPort, RouterScPort])`,
      B/IP `BACnetClient` routed read; assert counters, health `Up`, routing
      table contains both networks, and a hub restart drives health through
      `Reconnecting` back to `Up` with `reconnect_forever`.
- [x] Container scenario added as `run-w14-sc-ip-router.sh` and
      `compose.w14-sc-ip-router.yml` under `crates/rusty-bacnet/tests/container`
      following the W5 and W12 patterns, using Podman. W14 retains source,
      image, wheel, result, log, and cleanup evidence.
- [x] Update `docs/conformance/standard-135-2020-ledger.md` with the new
      evidence while retaining `implementation-present-needs-conformance-tests`;
      no broader support or conformance status is claimed.

Acceptance: all three tests green in CI; ledger row cites them.

## Phase 8 - release engineering (R9, R10)

- [x] Rename the distribution to `ace-rusty-bacnet` in
      `crates/rusty-bacnet/pyproject.toml` (project name, URLs pointing at the
      fork) while keeping the `rusty_bacnet` import name; confirm `maturin`
      emits `ace_rusty_bacnet-*.whl` containing the `rusty_bacnet` module and
      that `test_api_contract_manifest.py` still resolves the stub.
- [x] Reserve `0.12.1` as the first fork release on the upstream 0.12 line;
      the existing upstream `v0.12.0` tag is not reused for a fork build.
- [x] (2026-10-10: plain `vN` tags adopted, enforced by the fork release
      validation step; acceptable now that D1 is moot.) Define the fork tag scheme (recommendation: `v0.11.x-ace.N`) and make
      public release jobs trigger only on fork tags; production wheels must be
      built from those tags, and upstream `v*` tags fetched into the fork must
      not build or publish.
- [x] (2026-10-10: verified; v0.11.4 run built Linux x86_64/aarch64, macOS
      x86_64/arm64, Windows x86_64 and published to `ace-pypi`.) Extend `ci.yml` so fork tags build manylinux `x86_64` and `aarch64`
      wheels plus macOS wheels with `maturin-action` and attach them to a
      GitHub Release; gate `publish-pypi` on a repository variable so it
      cannot fire from the fork by accident.
- [ ] Replace the TODO at `ci.yml:475-480` with a multi-arch image job
      (`linux/amd64`, `linux/arm64`) for a router image built from the wheel,
      pushed to the fork's GHCR namespace on tags; local builds use
      `podman build --platform`.
- [x] Tag the first release once Phases 1-7 land. (2026-10-10: `v0.11.1`
      through `v0.11.4` tagged; `v0.11.4` is the first fully green release run.)

Acceptance: a tag produces downloadable wheels for both Linux architectures
and a multi-arch image manifest; `pip install` of the wheel passes
`test_sc_ip_router.py`.

## Proposed PR queue

| # | Title | Phase | Upstreamable | Size |
|---|---|---|---|---|
| 1 | transport: restore transport_kind/topology_id, add health and max_npdu hints | 1 | yes | M |
| 2 | sc: retry_forever and terminal health publication | 2 | yes | M |
| 3 | router: topology guard, health, routing snapshot, re-announce, oversize reject | 3 | yes | L |
| 4 | sc: configurable Max-NPDU/Max-BVLC on node and hub | 4 | yes | S |
| 5 | sc: shared dial helper and explicit runtime UUID | 5 | yes | M |
| 6 | python: typed router ports, from_ports, health, routing_table | 6 | yes | L |
| 7 | tests: mixed-transport router evidence and ledger update | 7 | yes | M |
| 8 | ci: fork wheel releases and multi-arch router image | 8 | fork-only | M |

PRs 1 and 2 are independent. PR 3 depends on 1. PR 5 depends on 2. PR 6
depends on 3, 4, and 5. PR 7 depends on 6.

## Validation gates

### G1 - per-PR Rust

```
cargo fmt --all -- --check
cargo clippy --workspace --all-features -- -D warnings
cargo test -p <crate> --all-features
```

### G2 - workspace

```
cargo test --workspace --all-features
```

### G3 - installed wheel

```
cd crates/rusty-bacnet && uv run maturin build --release
uv pip install target/wheels/ace_rusty_bacnet-*.whl
uv run pytest crates/rusty-bacnet/tests -q
```

### G4 - interoperability

Container scenarios W5 (virtual router) and the new W14 (SC-to-IP router)
pass under Podman.

### G5 - documentation

`rusty_bacnet.pyi`, `docs/python-api.md`, `docs/rust-api.md`, `CHANGELOG.md`,
and the ledger rows reflect the merged surface.

## Risks

- Upstream may land its own transport-health or router-config shape while
  this work is in flight. Mitigation: the per-PR upstream check in Working
  rules, and keeping PRs 1-3 small enough to rebase.
- Raising the SC Max-NPDU default changes what peers see in Connect-Request.
  Mitigation: verify the Annex AB value against the standard text before
  merging Phase 4 and keep the builder to override.
- Unbounded retry against a permanently misconfigured hub produces indefinite
  log noise. Mitigation: backoff capped at `max_delay_ms`, health exposes the
  attempt counter, and retry-forbidden NAKs still terminate.
- Removing the `attachment_id` UUID derivation breaks any existing
  `RuntimeScAttachment` caller. Mitigation: it is a required keyword with a
  clear `ValueError`, called out in `CHANGELOG.md` as a breaking change for
  the fork's next minor version.
- `from_ports` kwargs drifting from the router team's section 5.1. Mitigation:
  Phase 0 agreement before PR 6.

## Open decisions

1. Router-side restart of a failed port (`BACnetRouter.restart_port(index)`)
   is deferred behind health visibility. Confirm whether the router team needs
   it once `retry_forever` and `state = "failed"` are available.
2. Decided 2026-09-11: fork wheels ship as distribution `ace-rusty-bacnet`
   from fork tags. Still open: whether to publish to PyPI under that name in
   addition to GitHub Release assets, and the exact tag pattern.
3. Default for `announce_interval`. Recommendation: `None` (off) to match
   upstream traffic behavior, with re-announce on port recovery always on.
4. Whether the multi-arch image should ship a reference router entrypoint or
   only the wheel and Python runtime. Recommendation: wheel plus a minimal
   entrypoint that reads a TOML port list, so the image is useful on its own.

## Review 2026-09-12 - progress against plan

This section preserves the independent review of the original uncommitted
working tree. The resolution notes and checkboxes below track the subsequent
remediation and commit partitioning; unresolved items remain unchecked.

### Phase status

| Phase | Items done | Gap |
|---|---|---|
| 0 baseline | 4 of 4 | none |
| 1 transport identity and health | 6 of 6 | none |
| 2 unbounded SC reconnect | 5 of 5 | none |
| 3 router extensions | 6 of 6 | none |
| 4 SC size limits | 0 of 4 | not started |
| 5 shared dial helper and runtime UUID | 2 of 4 | shared helper/consolidation deferred |
| 6 Python typed ports | 5 of 6 | Phase 5 consolidation |
| 7 mixed-transport evidence | 5 of 5 | none |
| 8 release engineering | 4 of 5 | multi-arch image remains open |

Delivered against the router team's asks: R1-R6, R8, and R9. Open: R7 and R10.

### Verification performed by the reviewer

- `cargo test --workspace --exclude rusty-bacnet --locked --features
  bacnet-types/serde,bacnet-transport/ipv6,bacnet-transport/sc-tls
  --no-fail-fast` after `cargo clean -p bacnet-transport -p bacnet-network
  -p bacnet-integration-tests`: 48 suites, 4715 passed, 0 failed.
- `cargo fmt --all -- --check` and `git diff --check`: clean.
- Fresh debug wheel built with `uvx maturin build`, installed into a scratch
  venv outside the checkout. `test_virtual_router.py`,
  `test_router_ports.py`, and `test_sc_ip_router.py` (with its
  `test_sc_hub_mtls.py` fixture dependency): 8 passed, 0 failed.
- `router/tests/mixed_bip_sc.rs` asserts SNET/SADR insertion and hop-count
  in both directions as Phase 7 required.
- `rusty-bacnet` cannot be linked by `cargo test` on this host (missing
  `python3.11` static library); the `--all-features` build also fails in the
  third-party `gpiocdev-uapi` crate on macOS. Neither is caused by this work.

### Findings

- [x] **A1 - Split the working tree into the PR queue before committing.**
      The implementation is partitioned into dependency-ordered transport,
      network-router, runtime-identity, Python-surface, mixed-acceptance, and
      documentation commits. Transport and network changes remain isolated
      from fork packaging and documentation.
- [x] **A2 - Stale artifacts break the workspace build.** With the existing
      `target/`, `sc_ip_router` failed to compile against outdated transport
      and router rlibs (`TransportHealthState` unresolved, `port_health` and
      `routing_table` missing). A `cargo clean -p` of the three crates fixed
      it. The evidence README records the exact clean command and the isolated
      target used for the final workspace gate. Current CI restores
      `rust-cache`, so this is a local freshness proof rather than a claim that
      CI always starts without cached artifacts.
- [x] **A3 - Unify the unbounded-retry keyword name.** `RouterScPort` and
      `RuntimeScAttachment` now both use `reconnect_forever`, matching the
      existing `reconnect_*` kwarg family. Rust
      `ScReconnectConfig.retry_forever` remains unchanged.
- [x] **A4 - `announce_interval_s` is exposed but rejects every value except
      `None`.** Either land the Phase 3 `RouterConfig` and re-announce on
      port recovery, or drop the kwarg from `from_ports` until it works. Do
      not ship a public kwarg that only raises. The keyword is now absent and
      will return only with the Phase 3 `RouterConfig` implementation.
- [x] **A5 - R5 is silently downgraded.** `RuntimeScAttachment` now requires a
      keyword-only, nonzero 16-byte `device_uuid`, threads it through
      `ScConfig`, and no longer derives it from `attachment_id`. The shared
      dial helper remains a separate Phase 5 maintainability item.
- [x] **A6 - Rebase PR 2 against upstream's new SC redial work.** The refreshed
      pin is `0376fa3`, 14 commits after `b4c67ec`. Those commits add address
      resolution and direct dial/listener behavior and overlap `sc/mod.rs`,
      but do not change `sc/recovery.rs` or `sc/reconnect.rs`. A disposable
      cherry-pick of `715332d` onto `0376fa3` confirmed conflicts in
      `any.rs`, `port.rs`, and `sc/mod.rs`, plus modify/delete conflicts for
      the moved B/IP and virtual transports. Resolve those architecture moves
      on dedicated branch `proposal/sc-router-transport-health-retry`. Commit
      `e6e94d8` is based directly on `0376fa3`, preserves upstream direct SC
      discovery/listener behavior, and passes the full `bacnet-transport`
      `sc-tls` suite plus a workspace all-target check. The upstream proposal
      stays separate from this fork delivery stack.
- [x] **A7 - Ledger status is unchanged by design; say so to the router
      team.** `BACNET-6-ROUTER-MESSAGES` gained an evidence note but remains
      `implementation-present-needs-conformance-tests`, and
      `docs/conformance/support-summary.md` was not touched. This follows the
      plan's rule that acceptance tests are not conformance tests, but the
      router team's original ask expected "real evidence" for that row, so
      the distinction is stated here, in the changelog, and in the evidence
      README: this is implementation/interoperability evidence only, not a
      conformance or public support-status change.
- [x] **A8 - Rebuild the wheel before any Python acceptance claim.** The
      wheel in `target/wheels/` predated the source edits by eleven hours and
      lacked the new classes. The final macOS arm64 wheel and Linux arm64 W14
      wheel are both rebuilt from clean commit `af3c993`; their exact wheel
      and source/archive digests are recorded in the evidence README. The
      installed macOS suite and W14 acceptance both pass without skips.

### Recommended order

1. A3 and A4, since both change public kwargs that are cheapest to fix before
   the first commit.
2. A1 with A6 folded into PR 2 and A2 noted in the evidence README.
3. Phase 5 (A5), then the Phase 3 and 4 remainder for R7 and R8.
4. Phase 8 using the decided `ace-rusty-bacnet` distribution name, with A7
   and A8 reflected in the release notes and evidence.

## Review 2026-09-19 - combined branch closeout

This follow-up review covered the committed router stack from `9eed26a` through
`74cdb28` and the retained W14 evidence. The fixes remain within the delivered
R1-R6 scope; R7-R10 and the conformance-status caveat remain unchanged.

- [x] **B1 - Restore the strict source-file size gate.** SC transport state,
      topology, lifecycle reset, and reconnect-probe helpers now live in the
      focused `sc/transport_state.rs` module. `sc/mod.rs` is below the 700-line
      cap with headroom, and the strict CI script passes.
- [x] **B2 - Reject stale retained W14 wheels.** The runner extracts the wheel
      build metadata from the retained fixture image and compares its source
      archive digest with the current build-context snapshot before starting
      acceptance, including when `W14_SKIP_BUILD=1`.
- [x] **B3 - Enforce and prove the runtime SC UUID boundary.** Rust runtime
      configuration rejects an all-zero device UUID before certificate reads or
      network I/O. A loopback wire test starts the production configuration path
      and verifies the configured UUID bytes in the Connect-Request.
- [x] **B4 - Make W14 polling fail closed.** Marker waits now preserve failures
      instead of piping them through `tee`, the final container-exit wait is
      bounded, and the cleanup trap covers the temporary metadata container.
- [x] **B5 - Preserve the pre-dialed failover compatibility path.** A failed
      failover connector now falls back to an available preconfigured socket;
      a loopback test proves the fallback completes the SC handshake.
- [x] **B6 - Dial each Python router SC port at native port startup.** An
      additive deferred TLS WebSocket validates configuration immediately but
      performs its bounded dial on first I/O, so sequential native port startup
      can send each Connect-Request before the next SC port is opened. A
      two-listener regression proves construction dials neither endpoint and
      first I/O reaches the listeners strictly in port order.
- [x] **B7 - Reject overlapping SC endpoints.** Router collision checks compare
      normalized primary and failover endpoints independently, while preserving
      the combined display topology identity. Tests cover same-primary/different-
      failover and primary-to-failover overlap.

Current-tree verification: the full `bacnet-transport` SC/TLS suite, the
`bacnet-network` suite, runtime SC transport tests, router library build/tests,
formatting, diff checks, shell syntax/static checks, and the strict file-size
gate pass. W14 rebuilt the Linux arm64 wheel from the exact dirty-tree build
context digest `af6a1bf2432e3d59cda6cd45070045def1cb02c83273153edd0783eed354f20f`;
the retained metadata matches that digest, the post-hub-restart routing and
acceptance markers passed, exit status was zero, and cleanup passed. The wheel
digest is `45072672115d9475746d1f1b79c4dbb2ae6c0347c56d1dc8926ed0871ec95897`.
The installed macOS Python suite was not rebuilt for this remediation and
remains a publication gate.

## Review 2026-09-19 - independent verification of `fd9d5cc`

Reviewer verification of the B1-B7 closeout commit, run on macOS arm64 from a
cleaned target directory and a freshly built wheel.

| Check | Result |
|---|---|
| `cargo test --workspace --exclude rusty-bacnet --locked --features bacnet-types/serde,bacnet-transport/ipv6,bacnet-transport/sc-tls --no-fail-fast` | 48 suites, 4724 passed, 0 failed |
| `bash .github/scripts/check-file-size.sh` | pass |
| `cargo fmt --all -- --check`, `git diff --check` | clean |
| `bash -n` and `shellcheck -S warning` on `run-w14-sc-ip-router.sh` | clean |
| Fresh `maturin build` wheel, full Python suite from the checkout | 151 passed, 1 failed |
| Fresh final remediation wheel in an isolated CPython 3.13 environment with `PYTHONPATH` cleared | 153 passed, 1,877 subtests passed |

B1, B2, B3, B4, B5, and B7 are confirmed as described. B6 is correct in Rust
but changed a public Python contract; see C1.

### Findings

- [x] **C1 - `BACnetRouter.start()` SC dial failures now raise `BacnetError`,
      not `RuntimeError`.** B6 moved the SC dial from the binding into the
      native port start. A refused hub connection now surfaces through
      `to_py_err` as `rusty_bacnet.BacnetError` (base `Exception`) with the
      message `router port 1 (sc, identity "wss://...") failed to start: ...`.
      `test_sc_ip_router.py::test_start_error_names_sc_port_and_hub` asserts
      `RuntimeError` and fails against a fresh wheel. The message still names
      the port index and hub URL. Recommendation: map SC port start failures in
      `crates/rusty-bacnet/src/router.rs` back to `RuntimeError`, matching the
      documented client and server TLS/dial contract, and keep the test as-is.
      The B6 closeout text admitted the macOS Python suite was not rerun; the
      W14 scenario only runs `test_routed_read_health_routes_and_hub_recovery`,
      so this path had no coverage. The binding now maps native SC-port startup
      failures back to built-in `RuntimeError` while preserving native port,
      SC topology, and cause text; B/IP and other native startup failures retain
      the `BacnetError` taxonomy. The regressions assert both exact exception
      types, contextual details, and successful B/IP retry after bind release.
- [x] **C2 - Rerun the installed-wheel Python suite before closing any
      finding that touches `crates/rusty-bacnet`.** Add it to the closeout
      checklist alongside the Rust gates. A retained Linux W14 run is not a
      substitute because it executes one test. A fresh CPython 3.13 macOS arm64
      wheel was built from this branch, installed with `pytest` into a new
      temporary virtual environment with `PYTHONPATH` cleared, and the full
      `crates/rusty-bacnet/tests` suite passed: 153 tests and 1,877 subtests.
      The final wheel SHA-256 is
      `d2508d23b27fa634abc16c6bf40076584391a58e3d496ecc95e617723739b338`.
- [x] **C3 - Stop committing directly to `main`.** `fd9d5cc` is a single
      commit mixing transport, network, runtime, Python, and container
      changes, and nine commits now sit unpushed ahead of `origin/main`. The
      plan's PR queue and the per-layer rule in Working rules have not been
      followed for either slice. Existing local `main` history through
      `fd9d5cc` is retained as an integration baseline so reviewed source and
      artifact references remain stable. The mixed-layer commits are explicit
      historical exceptions, not upstream-ready PR units. All subsequent
      corrections are on `fix/sc-router-review-closeout`; upstream submissions
      will be extracted into separately validated, layer-specific branches.
      Neither local `main` nor remote history was rewritten or pushed.
- [x] **C4 - Re-audit the proposal branch against current `upstream/dev`.**
      The prior header recorded 112 upstream commits past the `0376fa3` pin with
      supersession unassessed. That audit gates PR 1 and blocks PRs 2 and 3
      from being rebased at all. The audit is now pinned to `3364a8d`, 119
      commits after the proposal base. Health watches/snapshots, topology
      collision identity, unbounded retry with hub alternation, router
      health/counters, detached route snapshots, runtime integration, typed
      Python ports, and W14 acceptance remain unique. Address resolution and
      direct-discovery foundations are upstream-owned. A replacement proposal
      must selectively preserve upstream reconnect jitter (`b0813a6`), the
      24-hour delay cap and persistent diagnostic throttle (`70dfcf4`),
      transport/origin provenance (`0645563`, `01c2d26`), NPDU admission
      (`7b20d3a`), and newer router admission/control/convergence behavior.
      The old `e6e94d8` proposal is retained as evidence but is not submission
      ready: it also lacks the corrected connector-to-preconfigured-failover
      fallback from `fd9d5cc`. Any replacement starts from the audited upstream
      tip on a fresh branch and is validated as layer-specific extracted work.
- [x] **C5 - Reconcile the file-size cap.** The CI script enforces 700
      non-empty, non-comment lines; the `aceiot-projects` guidance says 500.
      Repository-specific `AGENTS.md` now explicitly adopts the enforced
      700-line Rust cap, recommends comfortable headroom and cohesive splits,
      and clarifies that sibling projects' approximate 500-line convention is
      not this repository's gate. The strict CI script remains authoritative.

### Unchanged scope

R7 oversize handling, R8 re-announce, the Phase 5 shared dial helper, and all
of Phase 8 release engineering remain open. The conformance-status caveat from
A7 still applies.

## Review 2026-09-23 - closeout branch verification

Reviewer verification of `fix/sc-router-review-closeout` at `ab8e558` (six
commits over `main` `fd9d5cc`, pushed to `origin`), run on macOS arm64 from a
cleaned target directory and a freshly built wheel.

| Check | Result |
|---|---|
| `cargo test --workspace --exclude rusty-bacnet --locked --features bacnet-types/serde,bacnet-transport/ipv6,bacnet-transport/sc-tls --no-fail-fast` | 48 suites, 4724 passed, 0 failed |
| `bash .github/scripts/check-file-size.sh`, `cargo fmt --check`, `git diff --check main..HEAD` | clean |
| `shellcheck -S warning scripts/publish-python-release.sh` | clean |
| Fresh `ace_rusty_bacnet-0.11.1-cp313` wheel in a new venv with `PYTHONPATH` cleared, full Python suite | 153 passed, 1877 subtests, 0 failed |
| `rustls` in `Cargo.lock` | 0.23.45 as the changelog states |

C1 through C5 are confirmed closed. `test_start_error_names_sc_port_and_hub`
passes again, and the new B/IP taxonomy test passes alongside it. The
findings below concern the four release-engineering commits that rode along
on the same branch (`3c51b34`, `cb59f81`, `2788042`, `ab8e558`).

### Findings

- [x] **D1 - Git tag namespace overlap with upstream (closed 2026-10-10: upstream
      went straight to `v0.12.0` and never tagged a `v0.11.x`, so no clash exists).**
      The `ace-rusty-bacnet` distribution rename already prevents the artifact
      collision that open decision 2 was written to avoid: the private index
      holds only fork wheels, `publish-crates` and `publish-pypi` stay scoped to
      the upstream repository, and Rust consumers of the fork pull by git
      source, not by crates.io version. Plain `0.11.1` crate versions are
      therefore acceptable. What remains is that the `v0.11.1` tag lives in the
      same `refs/tags/` namespace upstream uses. Both remotes are on default
      auto-following tag fetch, so once upstream tags its own `v0.11.1`, any
      clone with both remotes fails `git fetch upstream` with a would-clobber
      error, and the CHANGELOG `[0.11.1]` heading will collide at the next
      reconciliation. Cheapest fix: set
      `git config remote.upstream.tagOpt --no-tags` in the documented setup and
      keep pinning upstream by SHA as the plans already do; optionally prefix
      fork tags (`ace-v0.11.1`) and adjust the one `refs/tags/v` prefix strip
      in the fork release step. Not a release blocker.
- [ ] **D2 - The fork wheel job can run with `validate` skipped.** `ab8e558`
      lets the release wheel job proceed on the fork when the cross-platform
      `validate` gate is skipped, provided the Linux, lint, audit, MSRV, and
      policy gates pass. This is a deliberate weakening of the inherited
      release gate, justified in a comment as tolerance for a flaky test. It
      should be recorded as an accepted risk in Open decisions with an owner
      and an expiry, or the flaky test should be fixed and the bypass removed.
- [ ] **D3 - The C1 fix keys off error text.** `router_start_to_py_err`
      classifies an SC start failure by matching the string prefix
      `router port {index} (sc, ` and the phrase `failed to start:` that
      `bacnet-network` formats at `router/mod.rs:71`. A wording change in the
      network crate silently reverts SC failures to `BacnetError`. The result
      is also a split taxonomy: SC port start failures raise `RuntimeError`
      while B/IP bind failures raise `BacnetError`, both now locked in by
      tests. Recommendation: have the native router return a structured start
      error carrying the port index and transport kind, map on that, and
      document the taxonomy in `rusty_bacnet.pyi` so callers know to catch
      both.
- [ ] **D4 - `pyproject.toml` hardcodes the private index.** `3c51b34` adds a
      `[[tool.uv.index]]` block pointing at a Tailscale hostname with
      `locked = true`. Anyone running `uv` inside `crates/rusty-bacnet` off
      the tailnet now resolves through an unreachable index, and the hostname
      ships in the sdist metadata. Move the index to a `uv.toml` or CI
      environment configuration and keep the publish URL in the release
      script only.
- [ ] **D5 - Phase 8 bookkeeping lags the commits.** `CHANGELOG.md` has a
      released `[0.11.1] - 2026-09-23` section and a tag exists, yet Phase 8
      still shows only the rename ticked. `ab8e558` also adds fork wheel
      building, which is most of the third item. Tick what is done, and note
      that the multi-arch image job at `ci.yml:475-480` is still a TODO.
- [ ] **D6 - Branch state.** The closeout branch is pushed, but `main` still
      carries nine unpushed commits and the branch is not merged. Open the PR
      from `fix/sc-router-review-closeout` to `main`, and decide whether the
      first fork release should be cut from `main` after merge rather than
      from an unmerged branch.

### Unchanged scope

R7 oversize handling, R8 re-announce, the Phase 5 shared dial helper, and the
multi-arch image job remain open. The conformance-status caveat from A7 still
applies.

## Review 2026-10-10 - release delivery and drift

Reviewer verification of `fix/sc-router-review-closeout` at `9f5e019` (seven
commits past `ab8e558`, all dated 2026-09-23: the private PyPI publish job,
cross-platform and Windows test fixes, and releases 0.11.2 through 0.11.4).
No commits on any branch since 2026-09-23.

| Check | Result |
|---|---|
| Workspace suite, CI feature set, cleaned crates | 48 suites, 4719 passed, 5 failed |
| The 5 failures (`cli_discovery`) rerun alone | 5 passed in 0.63s |
| `cargo fmt --check`, file-size gate, `git diff --check` | clean |
| GitHub run for tag `v0.11.4` | all gates green including `Validate Release` and `Publish to ACE PyPI`; no bypass path used |
| Private index `ace-pypi` | serves 0.11.4 wheels for Linux x86_64 and aarch64, macOS x86_64 and arm64, Windows x86_64 |
| `ace-rusty-bacnet==0.11.4` installed from the index into a clean venv, full Python suite | 153 passed, 1877 subtests, 0 failed |
| PR #4 to `main` | open, 22 commits, 87 files, 33 checks green, no review recorded |

The five workspace failures are all five tests in `crates/bacnet-cli/tests/
cli_discovery.rs` hitting the shared 10-second deadline in
`tests/support/mod.rs:17` at the same moment under parallel load. They pass
alone and on the macOS CI runner. This is the same flake class as the
runtime COV test noted on 2026-09-12, not a regression.

R9 is delivered. Phase 8 is complete except the multi-arch image job (R10).

### Findings

- [ ] **E1 - Merge PR #4.** (2026-10-11: now urgent. The same branch has
      since gained the embedded MS/TP and router work, about 100 commits and
      23,000 lines over `main`, so every later review inherits this unmerged
      base.) `main` is still at `fd9d5cc` locally, nine
      commits ahead of `origin/main`, and its own CI run from 2026-09-12 is
      red on the macOS and Windows test jobs. The branch's cross-platform
      fixes are what turned those green on the tag runs, so merging is also
      the path to a green `main`. This supersedes D6.
- [ ] **E2 - Remove the `validate`-skipped bypass now that it is unneeded.**
      The condition at `ci.yml:315` still allows fork wheels when the
      cross-platform gate is skipped. The flaky tests it was written around
      were fixed in `6b6e08ad`, `ceb9392b`, and `153dac52`, and `v0.11.4`
      passed the full gate. Delete the bypass and close D2.
- [x] **E3 - Upstream has moved a full minor.** `upstream/dev` is at
      `74fca30`, 1590 commits past the C4 audit pin, and upstream has tagged
      `v0.12.0`. The C4 audit and the upstream PR queue (PRs 1-3) are stale.
      Decide whether to keep offering R1-R6 upstream against 0.12, or to
      treat them as fork-only and open a new reconciliation plan for 0.12 in
      `docs/plans/`. Either way, stop citing the `0376fa3` proposal as current.
- [ ] **E4 - Pre-existing timing flakes under parallel load.** Two
      independent test groups now show the same pattern: the runtime COV
      observation test and the five CLI discovery tests. Both use fixed
      10-second deadlines. Worth a small ticket to make the deadlines
      load-tolerant or serialise those binaries, since they will keep costing
      reviewer time.

D3 (string-matched SC start error), D4 (private index in `pyproject.toml`),
and D5 (Phase 8 bookkeeping, now partly reconciled above) remain open.

### Unchanged scope

R7 oversize handling, R8 re-announce, the Phase 5 shared dial helper, and the
R10 multi-arch image job have not been touched since 2026-09-19. The
conformance-status caveat from A7 still applies.

## Reconciliation 2026-10-11 - upstream 0.12 continuation

The E3 decision is to continue R1-R6 upstream on the merged 0.12 router rather
than retain the 0.11 implementation as a fork-only line. The replacement work
starts at merge commit `da59a8a7` and preserves 0.12's `RouterOptions`,
`StartedRouter`, bounded discovery, routing-control policy, reconnect jitter,
24-hour delay cap, diagnostic throttling, provenance, and NPDU admission.

Delivered on `feature/sc-ip-router-upstream-0.12`:

- defaulted transport identity and health capabilities, including live SC
  health and unbounded primary/failover reconnect;
- duplicate-topology rejection, contextual port-start errors, ordered router
  health, and detached routing-table snapshots;
- frozen Python `RouterBipPort` and `RouterScPort` configurations plus
  `BACnetRouter.from_ports()`, stop-safe health/routes, and local B/IP address;
- restored initial-primary-error handoff so native SC startup can attempt the
  configured failover without weakening the 0.12 TLS policy.

The removed virtual-network transport, legacy virtual-router constructor, and
old per-port forwarding counters were not restored because the 0.12 native
router no longer owns those interfaces. R10 remains the next planned slice.

The first R7 slice is now delivered: SC exposes its negotiated encoded-NPDU
ceiling through the defaulted transport hint, and the router rejects an
oversize rewritten unicast before queueing it, increments `oversize_drops()`,
and returns Reject-Message-To-Network reason 4 to an addressable originator.
The exact-limit boundary forwards normally. R8 is also delivered through
`RouterConfig`, Python `announce_interval_s`, and per-port health watchers:
the periodic timer waits a full interval, while a non-`Up` to `Up` transition
immediately re-announces only on the recovered port. The integrated Phase 4
work sets the router-capable node/Hub default to 1497, exposes validated
Max-NPDU/Max-BVLC overrides, consolidates TLS transport construction, and
proves a maximum unsegmented 1476-byte B/IP APDU crosses the routed SC leg.

Verification on macOS arm64: `cargo test -p bacnet-transport -p
bacnet-network -p rusty-bacnet` passed (1,221 transport, 266 network, and 133
binding unit tests, plus integration targets). The all-features transport gate
remains Linux-only because its GPIO dependency does not compile on macOS.

## Review 2026-10-11 - upstream 0.12 continuation branch

Independent review of `feature/sc-ip-router-upstream-0.12` at `5ae0995`
(191 commits over upstream `dev` `bdac34b5`; upstream later moved through
`30da0ea3`). The closeout merged that upstream head at `3ad86663`, adopted
the upstream codec/core exactly, and retained the fork-provisional router
crates on top. Verified locally on macOS arm64 with the repository toolchain;
the bare-target router checks use the installed 1.97.1 toolchain.

| Check | Result |
|---|---|
| `cargo test --workspace --exclude rusty-bacnet --locked --features bacnet-types/serde,bacnet-transport/ipv6,bacnet-transport/sc-tls` | does not build: 0 suites ran |
| Fresh wheel from the branch, full Python suite in a clean venv | 526 passed, 3328 subtests, 0 failed |
| Embedded crates on the branch | upstream's merged codec and core with the fork's extended-frame series on top; `bacnet-router-core` and `bacnet-router-adapters` retained |
| Team's own verification note | transport, network and bindings crates only |

### Findings

- [x] **H1 - The workspace does not compile.**
      `crates/bacnet-endpoint/tests/bip_sc.rs:89` builds an `ScReconnectConfig`
      literal without the fork's `retry_forever` field. One line fixes it; the
      point is that the reconciliation was verified on three crates and the
      upstream `bacnet-endpoint` test target was never built. Run the full
      workspace command before pushing.
      Resolved by updating every complete `ScReconnectConfig` literal and by
      correcting the runtime mapping so `reconnect_forever` populates
      `retry_forever` without replacing the configured retry budget. The
      review's full workspace command now passes, including doctests.
- [x] **H2 - PR #5 gets no CI.** It targets `fix/sc-router-review-closeout`,
      and the workflow runs pull requests only against `dev` and `main`. No
      run exists for the branch. Retarget PR #5 at `main` (or stack it on PR
      #4 and merge that first) so the matrix, the no_std checks and the
      Python wheel job actually execute.
      Resolved by retargeting PR #5 directly to `main`; the branch now meets
      the workflow's pull-request base filter and has a full CI run.
- [x] **H3 - The fork release pipeline was lost in the merge.**
      `crates/rusty-bacnet/pyproject.toml` is back to `name = "rusty-bacnet"`
      at 0.12.0, the `ace-pypi` index block and the `publish-private-pypi`
      job are gone, and the wheel builds as `rusty_bacnet-0.12.0`. Restore
      the `ace-rusty-bacnet` distribution, the private publish job and the
      fork-tag gating from `3c51b34`, `ab8e558` and `be813fc7`, then decide
      the first 0.12-line fork version.
      Resolved with the ACE distribution name and index, fork-repository job
      gating, five-wheel private publish set, repaired locked sdist build,
      clean-venv smoke install, and `0.12.1` reserved as the first fork
      release on this line.
- [x] **H4 - The legacy constructor and `test_virtual_router.py` were
      dropped silently.** The reconciliation note says the virtual-network
      transport, the `BACnetRouter(bip_network, virtual_ports, ...)`
      constructor and the old per-port forwarding counters were not restored
      because 0.12 no longer owns them. That is a defensible pre-1.0 call,
      but it contradicts this plan's working rule and K5 of the embedded
      plan. Record it as a decision here, add a changelog fragment naming
      the removed API, and update `docs/python-api.md`.
      Resolved as an explicit pre-1.0 migration decision in this plan, the
      Python API guide, upgrade/install documentation, and changelog.
- [x] **H5 - R7 and R8 bookkeeping.** R8 is delivered (`RouterConfig`,
      `announce_interval_s`, recovery re-announce on the recovered port).
      The first R7 slice is delivered (negotiated SC NPDU ceiling as a
      transport hint, `oversize_drops`, Reject-Message-To-Network reason 4).
      Phase 3 and Phase 4 items should be ticked to match, with node and hub
      receive-limit configuration left open.
      Resolved: Phase 3 records both delivered slices. The concurrently
      integrated `4a3a0dc3` completes Phase 4's node/Hub receive-limit
      configuration and the 1476-byte B/IP-to-SC boundary evidence.
- [x] **H6 - Rebase before the gap grows again.** Upstream's new commits
      include a no_std B/IP and B/IPv6 BVLL codec (`bacnet-bip-codec`) and
      moved the Tokio transports onto it, a virtual-time MS/TP bus simulator,
      no-allocator firmware link images in CI, and a core fix that waits for
      a quiet line before every send. `bacnet-transport/src/bip` and
      `sc` are where this branch carries most of its delta, so each day of
      drift costs a merge.
      Resolved by merging upstream `dev` through `30da0ea3` (zero commits
      behind at closeout), restoring the codec/core byte-for-byte from that
      head, and adapting the provisional router adapter to upstream's merged
      extended-frame `MasterCore` and caller-owned action buffers. The merge
      also retained the fork's B/IP envelope provenance and contextual router
      startup errors across upstream's newer bounded ingress path.
- [x] **H7 - Upstream now owns the router adapters' B/IP side.** Issues
      #1622 (sans-IO B/IP node engine) and #1623 (BBMD tables and forwarding
      as a portable core) describe what `bacnet-router-adapters` does on
      the B/IP leg. Keep the adapters fork-provisional and point those issues
      at them rather than extending them further.
      Resolved by marking both router crates `publish = false`, documenting
      their provisional status, and linking PR #5 as reference evidence from
      upstream issues #1622 and #1623.

## Upstream integration 2026-10-12

Reviewer pass over upstream `dev` (`a82cb364`, 209 commits since the last
review) and the fork branch at `e1540af`. PR #5 now targets `main` and is
green (41 checks). Local verification with the pinned toolchain: 149 test
suites, 9695 passed, 0 failed; 21 doc-test suites, 94 passed. H1-H7 are
confirmed closed by `134f2460`, `7cc7a690`, `41c8f033` and the changelog
fragment `changelog.d/sc-router-legacy-api-migration.md`.

### What upstream did with the fork's work

- Extended frames (#1611) are done upstream: codec in #1650, core and host
  transport in #1666. The host MS/TP MAC was cut over to the core (#1608,
  `24f0db78`) with the U1-U6 regressions. #1617 and #1618 were fixed in
  `5cc188ad`. #1626 links no-allocator firmware images in CI. #1640 fuzzes
  the core. All of that is merged into this branch.
- #1678 ports two of the fork's router-adapter test cases upstream with
  credit; both are covered there already.
- #1665, #1696 and #1699 are upstream's own treatment of R7: reject an NPDU
  too long for the outgoing port with reason 4, refresh an SC port's limit
  after failover, and route the router's own traffic through the engine.
- #1679 is a portable sans-IO router core, `bacnet-router-core`, already
  merged on upstream `dev` (`ab013bda`) with `bacnet-network` depending on
  it. #1682 is the umbrella no_std program. #1684 is the 0.13.0 release
  checklist; 0.13 is "no_std-first" and ships when the milestone is done.
- #1663 asks for opt-in extended frames in `MstpEndpointBuilder` and the
  Python `MstpEndpoint`.

### Findings

Disposition update 2026-10-16: upstream tagged v0.13.0, merged #1663, #1665,
and #1696, and continued to `dev@488f7ecf`. PR #5 was fast-forwarded to
`main@e1540afc`, PR #4 was closed as superseded, and draft PR #6 was created
from upstream `dev` for the deliberate 0.13 port. Its first ACE release slice
is `66cdccaf`; the repaired sdist passed `cargo check --locked --offline` and
its CPython 3.13 macOS arm64 wheel passed an installed import smoke test.

- [x] **I1 - Crate-name collision on the next upstream merge.** Upstream
      `dev` now ships its own `crates/bacnet-router-core`, a different
      design (borrowed NPDU view, const-generic tables, `bacnet-time`), and
      `bacnet-network` depends on it. This branch still carries the fork's
      `bacnet-router-core` (publish = false) under the same path and name.
      The branch is 139 commits behind upstream and the collision lands with
      the next merge. Decide before merging: delete the fork crate and let
      `bacnet-router-adapters` target upstream's engine, or rename the
      fork's to `ace-router-core` and keep it provisional. Deleting is the
      option consistent with the adopt-upstream decision. **Disposition:** PR
      #6 starts from upstream `dev` and does not carry the fork crate or
      `bacnet-router-adapters`.
- [x] **I2 - Retire the fork's R7 slice in favour of #1665/#1696.** The
      fork's oversize rejection (`588a7be4`) and upstream's #1665 solve the
      same clause; #1696 adds the SC-limit refresh after failover that the
      fork does not have. When the branch merges upstream `dev` next, take
      upstream's implementation and keep only the fork tests that add
      coverage. Record R7 as delivered upstream, not fork-owned. **Disposition:**
      PR #6 uses upstream's implementation; only additional semantic test
      vectors may be ported.
- [x] **I3 - Merge PR #5 and close PR #4.** PR #4's branch has zero commits
      that PR #5 lacks. Close #4 as superseded, merge #5 to `main`, then
      merge upstream `dev` again immediately so `main` carries #1679 and the
      0.13 line before more fork work lands. **Disposition:** completed by an
      exact-head fast-forward of `main` to `e1540afc`; PR #4 is closed.
- [ ] **I4 - Align the fork release with 0.13.** `ace-rusty-bacnet` is
      restored on the branch at version 0.12.0 via `.github/workflows/
      release.yml`. Upstream's 0.13.0 checklist renames and first-publishes
      the no_std crates and adds an upgrading page. Plan the next fork
      release as a 0.13-based build after upstream tags, not a 0.12.x fork
      tag that upstream will never share. **Update:** v0.13.0 now exists and
      PR #6 is based on the post-release 0.13 `dev` line. Choose the first ACE
      0.13 version only after the runtime/router port and exact-head gates pass.
- [ ] **I5 - Repoint the firmware vendor.** `PROVENANCE.json` still pins
      the 0.11.7 fork commit with the pre-review codec and core. Vendor the
      branch head or upstream `dev` so the ESP32 build sees the reviewed
      core, extended frames, and `bacnet-time`.
- [x] **I6 - Python extended-frame opt-in (#1663).** The fork's Python
      router and MS/TP surface should expose the same opt-in flag upstream
      chooses, rather than a fork-only spelling. **Disposition:** upstream
      merged #1663 with `MstpEndpointBuilder::extended_frames(bool)` and
      Python `mstp_extended_frames=True`; PR #6 adopts that implementation.
- [x] **I7 - Reconcile the plan's K5 and Phase 4 wording.** Upstream deleted
      the host MAC and the `MasterNode` compatibility surface; the fork
      followed. Mark K5 and the Phase 4 adapter items in the embedded plan
      as superseded by #1608 so the plans stop describing an adapter that no
      longer exists. **Disposition:** reconciled in the embedded MS/TP plan.
- [x] **I8 - Keep PR #5 reviewable.** It is now the only integration path
      for roughly 200 fork commits plus two upstream merges. If `main` is
      not merged this week, split the next upstream merge into its own PR so
      the fork's own changes stay diffable. **Disposition:** PR #5 landed
      unchanged; the upstream 0.13 port is isolated in draft PR #6.

## Merge 2026-10-23 - upstream 0.14.0 onto the migration line

Branch `feature/upstream-0.14-migration` starts from the team's
`feature/upstream-0.13-migration` (`daddf3dd`), merges `main` (PR #5 and #7,
which the 0.13 branch lacked, restoring the three plan documents), then
merges upstream `dev` at `2d82e9a9` (v0.14.0 plus 20 commits). Zero commits
behind upstream at commit time.

### What was taken from upstream

- Everything the fork had not changed, which is almost all 2,200 modified
  files, including the rewritten `bacnet-network` router over
  `bacnet-router-core`, `bacnet-npdu`, `bacnet-bip-core`, the extended
  MS/TP host transport, the SC `ServerPolicy` and principal model, and the
  0.14 Python surface (SC events, reinitialize, log records).
- Upstream's own `bacnet-router-core` replaces the fork's crate of the same
  name; the fork's `bacnet-router-adapters` was already gone on the 0.13
  branch. I1 is closed by the merge.

### What the fork still carries (51 files, about 4,500 lines over upstream)

- The ACE Python router: `RouterBipPort`, `RouterScPort`,
  `BACnetRouter.from_ports`, `port_health`, `routing_table`,
  `announce_interval_s`. Upstream 0.14 added its own Python `BACnetRouter`
  with `BipRouterPort`, `ports()`, `routes()`, `close()` and an async
  context manager, B/IP only, no health. The fork's class keeps the name
  and the SC capability; upstream's module, stub classes and two router
  tests are not built. This is a deliberate divergence to raise upstream:
  propose SC ports and health on their `BACnetRouter`, then retire ours.
- `BACnetRouter::reannounce_port` in `bacnet-network`, re-added on
  upstream's new `Outputs` and `send_queue` structure (R8). The fork's
  `bip_endpoint` accessor is gone; the bound address comes from `ports()`.
- The runtime routed-property paths, the client property helpers, the ACE
  release workflow and scripts, and `validate_wss_uri` (now in
  `sc_tls/validate.rs`).

### Gates on macOS arm64, pinned 1.97.1 toolchain

| Check | Result |
|---|---|
| `cargo check`, workspace and bindings, all targets | clean |
| `cargo fmt --check`, 700-line gate | clean |
| Workspace tests, CI feature set | 268 suites, 12047 passed, 2 failed |
| The 2 failures on pristine upstream `dev`, same host | both fail there too (`bacnet-server` allocator fast-path size, `bacnet-cli` tshark pcap) |
| `cargo clippy -D warnings` | fails in `bacnet-bip-core` on 2 `must_use` lints, also on pristine upstream `dev` |
| Fresh `ace_rusty_bacnet-0.14.0` wheel, Python suite | 712 passed, 0 failed |

Environment notes for the team: Homebrew's `rustc` ahead of rustup on `PATH`
breaks cross-compiles, doc-tests and clippy with "incompatible version of
rustc"; put the 1.97.1 toolchain's `bin` first. `uv pip install` run inside
`crates/rusty-bacnet` consults the private index in `pyproject.toml` and
hangs off the tailnet (D4); install from another directory or pass
`--index-url`.

### Open after the merge

- [ ] **J1 - Upstream the SC-port and health extension of the Python
      router**, or adopt upstream's `BACnetRouter` and re-port. Until then
      the fork builds its own module and the two API shapes must not drift
      further apart.
- [ ] **J2 - `test_sc_ip_router.py` is gone.** Neither `main` nor the 0.13
      branch carries the installed-wheel SC router test that was the R2
      Python evidence; only `test_router_ports.py` (5 tests) remains. Restore
      it against the 0.14 surface.
- [ ] **J3 - Report the two upstream-side failures and the clippy lint
      upstream** if they reproduce on Linux CI; on this host they are
      pre-existing.
- [ ] **J4 - PR #6 is superseded.** Open a PR from
      `feature/upstream-0.14-migration` to `main` and close #6.
- [ ] **J5 - I2, I4, I5, I7 and I8 still apply** (retire the fork's R7
      slice for #1665/#1696, 0.13-plus release line, firmware vendor pin,
      plan wording for K5/Phase 4, PR hygiene).

## Out of scope

- Changes to the SC hub relay semantics beyond configurable size limits.
- Sibling repositories that consume the wheel.
- New GitHub workflows beyond extending `ci.yml`.
- BBMD or foreign-device behavior on the B/IP port.
