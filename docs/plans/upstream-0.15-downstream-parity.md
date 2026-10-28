# Upstream 0.15 integration: downstream acceptance

The integration merges upstream `dev` through
`0ac1fc102fd466c7c101f6eef7196c2070a6c6f1` into the ACE migration base
`788e4d28e3ec2feb25a906c9320c834948f4d332`. The first checkpoint was
`773c4025e116b27b5d875921faee6290dfd9d565`; the follow-up imports
upstream SC direct-link heartbeat, accepted-link idle, and hub probe-liveness
work without replacing the ACE mixed router.

Upstream still declares version `0.14.0`; this branch name is not a release.

## Integration changes

Upstream `BACnetRouter` and its B/IP application client/server ownership now
coexist with `AceMixedRouter`. ACE consumers, including `ace-sc-adaptor`, must
replace `BACnetRouter.from_ports(...)` with `AceMixedRouter.from_ports(...)`
and update imports. This class rename is an integration-introduced break.
The mixed router retains `RouterBipPort`, `RouterScPort`, `start`/`stop`, health
and routing snapshots. Upstream uses `BipRouterPort` and `close`; these APIs
are not interchangeable.

The mixed owner now retains startup and cleanup across waiter cancellation.
Stop failures raise `RuntimeError`, remain observable on later stop/start,
and prevent restart. Native cleanup failures are terminal for that object;
the native router does not retry failed cleanup. Recovery notifications
received before the SC monitor is spawned or first polled remain observable.
An unread initial Connected state can cause one duplicate startup announcement.
A stop admitted before startup publishes can make `start()` raise
`RuntimeError("router startup was cancelled")`; awaiting stop remains the
cleanup barrier whether startup succeeded or was cancelled.

## Pre-existing gaps and downstream work

- Mixed-router SC failover remains rejected at configuration time. Supporting
  it requires preserving the active hub's identity throughout initial dial,
  reconnect and failover, with end-to-end evidence.
- `port_counters()` was already absent before this integration. Router-wide
  counters cannot substitute for genuine per-port counts. SC health currently
  exposes state but not complete hub/error/retry/elapsed-state details.
- `bacnet-runtime` remains B/IP-only. Restoring workspace membership does not
  implement runtime SC attachments or broaden transport support.
- A separate read-only review identified pre-existing runtime failure-state
  risks that remain unresolved here: an operation-drain timeout after
  `stop()` sets `stopped=true` can leave attachments running while later stop
  calls no-op; `reconcile()` removes the active map before cleanup and can
  return from cleanup failure without restoring it; failure of both desired
  startup and rollback can leave an empty map with `stopped=false`. Commands
  can then be admitted only to fail with attachment-not-found. These findings
  are release risks, not integration regressions or evidence of wider runtime
  support. Add regression tests for socket/task release after drain timeout,
  retryable cleanup, map restoration after stop failure, and explicit failed
  state after unsuccessful rollback before treating those paths as accepted.
- Node F still needs a separate comparison against its vendored 0.11.7 line:
  reconcile its deadline and parser patches before updating that vendor pin.
  This integration makes no changes to Node F or other downstream projects.
- The installed-wheel mixed-router tests exercise B/IP lifecycle and SC
  configuration validation. They do not establish mixed SC wire forwarding,
  reconnect, failover or oversize behavior.

## Acceptance gates

| Gate | Evidence / remaining requirement |
|---|---|
| Both Python router APIs | Fresh CPython 3.13 wheel passed `test_router.py`, `test_router_ports.py`, and `test_router_application.py` after repair: 21 tests and 22 subtests. The mixed suite then passed again with UDP-port release and successful restart asserted after concurrent stop. |
| Runtime and native routing | Runtime 19, network 329 and router-core 14 unit tests passed; recovered/unknown-port traces, hosted reannouncement and routed-RPM helper passed. |
| Mixed lifecycle failure/cancellation | All six injected native tests passed after repair: cleanup failure/restart fence, cancelled multiport rollback, cancelled stop barrier, lost startup publication, and SC recovery before monitor spawn or first poll. The 13 upstream router unit tests and 12 lifecycle integration tests also passed unchanged. |
| Follow-up upstream SC integration | `cargo check --locked` passed for runtime, bindings, transport and ops. Transport passed 1,430 tests with one pre-existing local B/IP shared-port test excluded; that test also timed out on the pre-merge checkout. A fresh CPython 3.13 wheel passed 55 router/SC-hub Python tests and 357 subtests. Ops and MCP passed 243 and 129 unit tests. |
| Runtime failure-state release risk | Resolve and test the pre-existing stop/drain-timeout and reconcile/rollback findings above; the current 19 passing runtime tests do not establish these failure paths. |
| ACE adaptor migration | Update imports/construction and run adaptor tests against the exact built wheel. Resolve required missing counters and health fields without synthetic values. |
| SC wire behavior | Run an exact-wheel Python 3.13 SC hub + mixed-router + B/IP device scenario, including read/write, reconnect, teardown and oversized-message handling. Record the wheel hash and fixture configuration. |
| Node F vendor update | Compare 0.11.7 deadline/parser patches, preserve needed behavior, then run firmware and protocol regressions before changing the pin. |
| Release build | Build and install the intended Linux wheels with the pinned toolchain, run exact-wheel Python 3.13 and SC/oversize gates, and validate private `ace-rusty-bacnet` packaging before any release. |

Local integration checks used Homebrew Rust 1.98.1 on macOS arm64; upstream
pins Rust 1.99.0. Cargo test was used because nextest was unavailable.
The follow-up local wheel is
`ace_rusty_bacnet-0.14.0-cp313-cp313-macosx_11_0_arm64.whl`, SHA-256
`f04970c2d8603e45b5f38162d390e8b818587ac28d56e5ec318192ef00b337d8`.
Targeted Clippy with `--no-deps` and `-D warnings` passed for runtime,
bindings, transport, ops and MCP. The prior parallel binding test run exposed
a timing-sensitive logging assertion; all 214 binding unit tests passed with
one test thread.
Dependency Clippy reported unchanged upstream `must_use_candidate` warnings
in `bacnet-bip-core` (`bbmd/fdt.rs:274`, `bbmd/forward.rs:113`). These local
checks do not replace pinned-toolchain CI or release approval. No wheel was
published and no downstream acceptance gate is claimed complete here.
