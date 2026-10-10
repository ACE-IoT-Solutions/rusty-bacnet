# Embedded MS/TP work plan

Status: active; decisions 1-4 ratified and Phases 0-1 implemented 2026-10-10
Created: 2026-10-10
Target line: `fix/sc-router-review-closeout`; Phase 0 released at `7f7acfac`
(`v0.11.5`). The firmware still vendors the `9f5e019`/0.11.4 starting point;
rebase onto `main` once PR #4 merges.
Request sources: `ace-router-debug/firmware/rust-espidf/upstream-requests/`
`0001-no-std-wire-codecs.md` and `0002-portable-mstp-core.md`
Downstream consumer: `ace-router-debug/firmware/rust-espidf`, which vendors
this repository at `9f5e019` (`vendor/rusty-bacnet/PROVENANCE.json`) and today
builds only `bacnet-types` because it is the only crate with a `no_std` mode
Related plans: `docs/plans/sc-ip-router-workplan.md` (finding E3 on upstream
drift), `docs/plans/upstream-reconciliation-workplan.md`

## Goal

Let the ESP32-S3 node F firmware reuse this repository's tested MS/TP frame
codec and master state machine without compiling the host transport runtime,
and let an ESP32-P4 passive observer check every emitted frame with the same
code. Both capabilities must build for targets without `std`, allocate nothing
on the routing hot path, expose bounded and observable queues, and leave the
existing Linux and Python MS/TP API working through adapters.

The two requests are tracked here as K1-K5 (codecs) and M1-M8 (master core):

| ID | Request |
|---|---|
| K1 | `bacnet-encoding` and `bacnet-services` build with `no_std + alloc` |
| K2 | MS/TP frame types, CRC-8, CRC-16, incremental decode, bounded encode in a crate that builds with `default-features = false` |
| K3 | Caller-provided output storage so the hot path does not allocate |
| K4 | COBS and CRC-32K extended frames rejected explicitly and feature-tested, never silently accepted |
| K5 | Host API preserved through adapters |
| M1 | Synchronous event-driven core: `on_octet`, `on_silence`, `on_deadline`, `enqueue_npdu`, `poll_action` |
| M2 | Injected monotonic clock; the core returns deadlines and never sleeps |
| M3 | Actions `Transmit`, `SetDirection`, `DeliverNpdu` so platform code owns UART, DMA, and turnaround |
| M4 | Independently bounded, observable unicast and broadcast queues with priority, starvation limit, expiry, and explicit rejection reasons |
| M5 | All Clause 9 timers, `Max_Master`, `Max_Info_Frames`, retry limits, and multiple-frame ratio in a validated configuration |
| M6 | No allocation after initialization; counters for transitions, deadline misses, token rotation, regeneration, retries, parser errors, queue depth and drops, DE assertion duration |
| M7 | Tokio adapter keeps the existing Linux API |
| M8 | Core holds no platform or diagnostic locks and makes no system calls |

## Baseline assessment

### What exists on `9f5e019`

- `bacnet-types` is already `no_std` behind a default `std` feature
  (`crates/bacnet-types/src/lib.rs:14-17`). Its `Error::Transport(std::io::Error)`
  variant is gated on `std` (`error.rs:106-108`).
- `bacnet-encoding` (27 files, about 10,200 lines) has no feature flags and
  uses `std` unconditionally in exactly one place: the `HashMap` in
  `src/segmentation.rs:9,84`. Everything else is `bytes`, `Vec`, `String`,
  and two `tracing` call sites.
- `bacnet-services` (68 files, about 23,800 lines) has no feature flags, no
  `std::` paths, no maps, about 690 `bytes` references, 75 `Vec`, 18
  `String`, and two `tracing` call sites. The migration is mechanical.
- Host-target `cargo check --no-default-features` passes for all three crates,
  but that does not prove `no_std`. The first real bare-target checks failed
  because `bacnet-types` inherited `thiserror/default -> thiserror/std`.
  Phase 0 therefore includes repairing that existing feature wiring before
  seeding CI with `bacnet-types`; encoding and services remain Phase 1.
- The MS/TP frame codec is `crates/bacnet-transport/src/mstp_frame.rs` (514
  lines) with 47 tests in `mstp_frame_tests.rs`. It depends only on
  `bacnet_types::error::Error` and `bytes`. It already rejects COBS frame
  types 32 to 127 (`is_unsupported_cobs_frame_type`, `:107`) but through a
  `format!`-built `Error::Encoding`, which needs `alloc`. `decode_frame_stream`
  and `retain_lone_preamble_byte` operate on `Vec<u8>`.
- `bacnet-transport` is the problem the request describes: `lib.rs` exposes
  `bip`, `bbmd`, `bvll`, `sc`, `sc_frame`, `loopback`, `virtual_network`, and
  `port` unconditionally, and `Cargo.toml` pulls `tokio`, `socket2`, `libc`,
  `getrandom`, and `windows-sys` unconditionally. Only `mstp_serial`, `bip6`,
  `ethernet`, and the SC TLS pieces are feature-gated.
- The master state machine is `crates/bacnet-transport/src/mstp/mod.rs` (665
  lines) plus the Tokio driver `mstp/port.rs` (799 lines). `MasterNode` is
  already mostly synchronous (`handle_received_frame`, `use_token`,
  `done_with_token`, `pass_token`, `pass_token_timeout`, `poll_timeout`,
  `queue_npdu`) but holds `VecDeque<(u8, Bytes)>` with a fixed
  `MAX_TX_QUEUE_DEPTH` of 256 (`:77`), a `tokio::sync::oneshot` reply receiver,
  and `tokio::sync::mpsc`. Timers are `const` values (`:45-64`), not
  configuration. There is no counters struct. `port.rs` drives it with
  `tokio::select!`, `tokio::time`, and an `Arc<Mutex<MasterNode>>`.
- DE/RE handling lives in `mstp_serial.rs` (310 lines) and
  `mstp_serial/kernel_rs485.rs` behind the `serial` and `serial-gpio`
  features: hardware auto, Linux `TIOCSRS485`, and GPIO character device.
- Existing master tests: 41 in `mstp/tests.rs`, 12 in `clause956_tests.rs`,
  3 in `port_timing_tests.rs`, 2 in `reply_tests.rs`, plus 4 host assembly and
  abort tests in `mstp/port.rs`. All 62 are the regression floor for the
  adapter.
- Consumers of the MS/TP API that must not break: `crates/rusty-bacnet/src/
  mstp_py.rs`, the Python client, server, and router lifecycles, the CLI
  transport selection, and `AnyTransport::Mstp`.
- Host inter-byte silence is already documented as an approximation
  (`mstp/mod.rs:69`): host async reads cannot see `T_frame_abort`. The core
  can model it exactly because the platform delivers octets with timestamps.

### Facts that shape the design

- The S3 firmware target is `xtensa-esp32s3-espidf`, which has `std`. The
  `no_std` requirement is therefore about dependency hygiene (no Tokio,
  sockets, `libc`, OS serial) and about the P4 observer and any future
  bare-metal build, not about the S3 image alone. Acceptance must use real
  `no_std` targets, not `--no-default-features` on the host.
- `bytes` supports `no_std` with `default-features = false`; `tracing` does
  too but needs care; `thiserror` 2 supports `no_std`. The workspace already
  uses compatible versions.
- The workspace enforces a 700-line file cap, `missing_docs = "warn"`,
  `unsafe_code = "deny"`, and MSRV 1.93.
- Upstream `dev` was `74fca307` on 2026-10-10, 1729 commits past the fork's
  `9f5e019` base. New crates carry little conflict surface; changes inside
  `bacnet-transport/src/mstp` carry a lot.

## Working rules

- Everything the firmware consumes lands in new crates with `#![no_std]` at
  the root and `std` and `alloc` as opt-in features. Nothing in those crates
  may depend on `bacnet-transport`.
- The existing public paths `bacnet_transport::mstp_frame::*`,
  `bacnet_transport::mstp::{MstpTransport, MstpConfig, SerialPort,
  LoopbackSerial, NoSerial}`, and `AnyTransport::Mstp` keep compiling for
  current callers. Deprecations are allowed, removals are not, in this plan.
- The host `Bytes`-based API stays as an adapter layer over slice-based
  primitives. Do not rewrite the 690 `bytes` uses in `bacnet-services`.
- The core crates never call `std::time`, never spawn, never lock, never
  log. Diagnostics are counters the platform reads.
- Every acceptance claim about `no_std` is proved by a cross-compile for a
  target without `std` in CI, plus a dependency-tree assertion.
- Golden behavior is proved by differential tests against the current codec
  and state machine on the same inputs, before the old code is replaced.
- One PR per crate or adapter, in dependency order. File the two upstream
  issues from the request texts at kickoff so upstream can object to the
  shape before code exists.
- Keep every file under the 700-line cap from the first commit; the core
  crate is split by concern, not grown then split.
- Hardware validation on node F and the P4 observer is ACE's acceptance and
  is tracked here as a handoff, not as repository work.

## Architectural decisions

| Boundary | Decision | Rationale |
|---|---|---|
| Crate layout | Two new crates: `bacnet-mstp-codec` (K2-K4) and `bacnet-mstp-core` (M1-M8); both `no_std` by default, features `alloc`, `std`, `heapless` | Codec is useful alone to the P4 observer; core depends on codec; neither depends on transport |
| Codec storage (K3) | Encode into caller `&mut [u8]` returning the written length; decode from `&[u8]`; a small `FrameSink` trait with impls for `&mut [u8]`, `heapless::Vec<u8, N>` (feature), and `BytesMut` (std) | Covers the three storage styles the request names without a generic explosion |
| Incremental decode (K2) | `StreamDecoder` with fixed internal state and a caller-owned data buffer; `push(byte) -> Event` and `push_slice` | Byte-at-a-time matches UART ISR delivery; slice form matches DMA chunks |
| Codec errors | `FrameError` enum with no payload strings; `From<FrameError> for bacnet_types::Error` under `alloc` | Removes `format!` from the hot path and keeps host error text identical through the adapter |
| COBS (K4) | `FrameType::is_cobs()` and `FrameError::UnsupportedCobsFrame` with exhaustive tests for every type in 32..=127; do not publish a Cargo `cobs` feature until support exists | Explicit rejection today without making `--all-features` intentionally uncompilable |
| Core API (M1-M3) | `MasterCore<Q: QueueStorage>` with `on_octet(byte, now)`, `on_silence(now)`, `on_deadline(now)`, `on_transmit_complete(now)`, `enqueue_npdu(dest, npdu, class, now)`, `poll_action()`, `next_deadline()`; `Action::{SetDirection(Level), Transmit(&[u8]), DeliverNpdu{..}, Idle}` | The platform copies a borrowed transmit slice into UART/DMA storage before releasing the borrow, then reports completion so DE can be lowered; `poll_action` drains a fixed action ring |
| Clock (M2) | `Instant` and `Duration` from a caller-supplied `Monotonic` trait with microsecond resolution; the core stores absolute deadlines | Avoids `std::time`, lets ESP timers and simulated time share one path |
| Queues (M4) | `QueueStorage` trait with two impls: `HeaplessQueues<const U: usize, const B: usize, const N: usize>` and `AllocQueues` (`alloc` feature); per-queue depth, high-water, drop counters by reason `{Full, Oversize, Expired, Starved}`; broadcast-to-unicast ratio and max-age in config | Bounded by construction on embedded, unchanged depth on host |
| Configuration (M5) | `MstpCoreConfig` with every Clause 9 timer, `N_retry_token`, `N_min_octets`, `N_poll`, `Max_Master`, `Max_Info_Frames`, multiple-frame ratio, baud; `validate()` enforces standard bounds and returns a typed reason | Replaces the `const` timers at `mstp/mod.rs:45-64` |
| Counters (M6) | `MstpCounters` plain struct of `u32` fields, `Copy`, read by `counters()`; DE assertion duration accumulated from `SetDirection` pairs using the injected clock | No atomics or locks inside the core (M8) |
| Host adapter (M7, K5) | `bacnet-transport::mstp` becomes a Tokio driver over `MasterCore<AllocQueues>`: the read loop feeds `on_octet`/`push_slice`, `sleep_until(next_deadline)` feeds `on_deadline`, actions map to serial writes and the existing DE/RE modes | Keeps `MstpTransport<S>` and the 58 host tests as the regression floor |
| `bacnet-encoding` and `bacnet-services` (K1) | Add default `std` feature, `#![cfg_attr(not(feature = "std"), no_std)]`, `extern crate alloc`; `bytes` with `default-features = false`; `tracing` gated behind `std` with no-op macros otherwise; `segmentation` uses `alloc::collections::BTreeMap` | Smallest diff that satisfies the request |
| Embedded CI | New `embedded-check` job: `thumbv7em-none-eabi` (bare Cortex-M, no `std`) and `riscv32imac-unknown-none-elf` (closest tier-2 target to the P4) for `bacnet-types`, `bacnet-encoding`, `bacnet-services`, `bacnet-mstp-codec`, `bacnet-mstp-core`; a script asserts no `tokio`, `socket2`, `libc`, `tokio-serial`, or `tracing` in `cargo tree -e normal` for the two new crates | Proves the acceptance items the request lists |
| Property testing | `proptest` as a dev-dependency of the two new crates; `cargo-fuzz` targets under `fuzz/` are added but not run in CI | Deterministic in CI, deeper locally |

## Phase 0 - freeze inputs and file upstream issues

- [x] Record `main` or the closeout branch SHA this plan starts from, and the
      firmware's `PROVENANCE.json` commit, in this header.
- [x] File the two upstream issues using the request texts verbatim, and ask
      upstream whether they prefer new crates or feature-isolated modules
      before Phase 2 starts: codec request
      [#1604](https://github.com/jscott3201/rusty-bacnet/issues/1604) and
      portable-core request
      [#1605](https://github.com/jscott3201/rusty-bacnet/issues/1605).
      Upstream endorsed the new-crate boundary in the
      [#1604 maintainer response](https://github.com/jscott3201/rusty-bacnet/issues/1604#issuecomment-6099513881)
      and applied that answer to #1605.
- [x] Repair `bacnet-types`' existing `no_std` feature wiring by disabling
      `thiserror` default features; prove the fix on both real bare targets.
- [x] Add `thumbv7em-none-eabi` and `riscv32imac-unknown-none-elf` to the
      documented toolchain setup and to CI; land the `embedded-check` job
      with only `bacnet-types` in scope so it is green from day one.
- [x] Capture the baseline: the 47 codec tests and 62 master/adapter tests pass;
      record counts in `docs/plans/evidence/embedded-mstp/README.md`.
- [x] Version line settled (decision 4): this work lands on the fork's
      0.11.x line; a 0.12 reconciliation is not a prerequisite.

Acceptance: issues filed, embedded CI job green on `bacnet-types`, baseline
recorded. Satisfied by `v0.11.5` CI run
[`38064050799`](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/38064050799).

## Phase 1 - `no_std + alloc` for encoding and services (K1)

Crates: `bacnet-encoding`, `bacnet-services`.

- [x] `bacnet-encoding`: add `std` default feature and `alloc` plumbing;
      replace the `HashMap` in `segmentation.rs` with `BTreeMap`; gate
      `tracing` behind `std`; `bytes` with `default-features = false`.
- [x] `bacnet-services`: same feature shape; replace `std` prelude imports
      with `alloc` equivalents.
- [x] Add both crates to the `embedded-check` job with
      `--no-default-features --features alloc`.
- [x] Confirm `bacnet-objects` and `bacnet-network` still build with default
      features and that no public type changed.

Tests: existing suites unchanged; a doc test per crate showing the `no_std`
import path.

Acceptance: both crates cross-compile for both embedded targets; workspace
suite unchanged. Local acceptance passed on 2026-10-10; remote acceptance is
recorded by CI run
[`38067493332`](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/38067493332)
and docs run
[`38067493341`](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/38067493341).

## Phase 2 - `bacnet-mstp-codec` (K2, K3, K4)

Crates: new `crates/bacnet-mstp-codec`; adapter changes in `bacnet-transport`.

- [x] Create the crate with `#![no_std]`, depending only on `bacnet-types`
      with `default-features = false`; features `alloc`, `std`, and
      `heapless`. Do not expose `cobs` until it is implemented.
- [x] Move `FrameType`, `MstpFrame` header type, constants, CRC-8 and CRC-16
      with their tables, and validation from `mstp_frame.rs`; keep the
      numeric values and residual constants byte-for-byte.
- [x] Add `encode_into(&mut [u8]) -> Result<usize, FrameError>` and the
      `FrameSink` trait; add `StreamDecoder` with `push` and `push_slice`
      and a bounded caller buffer; keep `decode_frame(&[u8])` for one-shot
      use.
- [x] `FrameError` without payload strings; `From<FrameError>` into
      `bacnet_types::Error` under `alloc` producing the current message text.
- [x] COBS: explicit rejection for every raw type in 32..=127 and for data
      CRC-32K framing. Keep the unsupported behavior feature-independent.
- [x] `bacnet-transport::mstp_frame` becomes re-exports plus `Bytes`-based
      wrappers (`encode_frame`, `decode_frame_stream`,
      `retain_lone_preamble_byte`) so current callers compile unchanged.
- [x] Tests: port the 47 codec tests; add a differential test feeding the
      same corpus to the old `mstp_frame` (kept under `#[cfg(test)]` until
      Phase 5) and the new codec; `proptest` cases for truncation, oversize
      length, header and data CRC corruption, preamble resync, and sink
      exhaustion; `fuzz/fuzz_targets/mstp_decode.rs`.
- [x] Dependency-tree script passes for the codec with every feature
      combination.

Acceptance: codec cross-compiles for both embedded targets with no features;
differential test shows identical accept and reject decisions and identical
bytes for every golden frame; host transport tests unchanged. Local acceptance
passed on 2026-10-10; the `v0.11.6` tag run is the remote release gate.

## Phase 3 - `bacnet-mstp-core` (M1 to M6, M8)

Crates: new `crates/bacnet-mstp-core`.

- [ ] `MstpCoreConfig` with every Clause 9 parameter and `validate()`; port
      the existing `MasterNode::new` checks and add the standard ranges.
- [ ] `Monotonic` trait, `Instant`, `Duration`; a `SimClock` test helper.
- [ ] `MasterCore` state machine ported from `mstp/mod.rs`, keeping the
      `MasterState` names and Clause 9.5.6 transitions; `on_octet` runs the
      codec `StreamDecoder` and models `T_frame_abort` exactly.
- [ ] `QueueStorage` with heapless and alloc impls; class, priority,
      starvation limit, expiry, and typed rejection reasons.
- [ ] `Action` ring and `poll_action`; `SetDirection` emitted before every
      transmit and after the last octet is accepted by the platform.
- [ ] `MstpCounters` with the fields the request names.
- [ ] Split modules so no file exceeds the cap: `config`, `clock`, `queue`,
      `state`, `actions`, `counters`.
- [ ] Tests: deterministic simulated-time tests for every master state and
      every deadline edge, ported from `clause956_tests.rs` and `tests.rs`;
      queue overflow proves token passing continues; `proptest` on random
      event interleavings asserting no panic, no allocation after `new`
      (checked with a counting allocator under `alloc`), and monotone
      deadlines.
- [ ] Dependency-tree script passes; `#![forbid(unsafe_code)]`; no `std`,
      no `tokio`, no locks.

Acceptance: core cross-compiles for both embedded targets with `heapless`
only; all simulated-time tests green; counters observable in every test.

## Phase 4 - Tokio adapter over the core (M7, K5)

Crates: `bacnet-transport`.

- [ ] Rewrite `mstp/port.rs` to drive `MasterCore<AllocQueues>`: serial reads
      feed `push_slice` with timestamps, `tokio::time::sleep_until` feeds
      `on_deadline`, `poll_action` maps to serial writes and to the existing
      DE/RE modes in `mstp_serial.rs`.
- [ ] Keep `MstpTransport<S>`, `MstpConfig` (now converting into
      `MstpCoreConfig`), `SerialPort`, `LoopbackSerial`, `NoSerial`,
      the existing `node_state() -> Option<&Arc<Mutex<MasterNode>>>` contract,
      and `AnyTransport::Mstp` source-compatible. Retain `MasterNode` as a
      deprecated adapter wrapper and add a separately named snapshot API;
      changing `node_state()` itself would break current callers and tests.
- [ ] Expose `counters()` on `MstpTransport` and through
      `RouterPortCounters` where it fits.
- [ ] Python: `mstp_py.rs` and the client, server, and router lifecycles
      compile unchanged; add `counters` to the Python MS/TP surface only if
      the stub change is trivial, otherwise defer.
- [ ] Delete the old `MasterNode` implementation internals and differential
      scaffolding while retaining the deprecated compatibility wrapper.

Tests: the 62 host master/adapter tests pass unchanged; `test_mstp_compat.py` passes
on an installed wheel; a new adapter test proves a `SetDirection` pair
brackets every transmit with the GPIO mode.

Acceptance: host behavior unchanged; workspace suite and Python suite green.

## Phase 5 - feature matrix, docs, and release

- [ ] `embedded-check` covers both targets and the feature matrix: `no_std`,
      `no_std + alloc`, `no_std + heapless`, `std`, and the Tokio adapter.
- [ ] `.github/scripts/check-embedded-deps.sh` asserts the dependency tree
      for both new crates and runs in the fast gate.
- [ ] `docs/rust-api.md`: new "Embedded MS/TP" section with a bare-metal
      codec example, a core driving loop example, and the counters table.
- [ ] `CHANGELOG.md` entries; version bump per the Phase 0 decision; fork
      release through the existing `ace-pypi` path so the Python wheel picks
      up the adapter.
- [ ] Update the two upstream issues with the PR links.

Acceptance: CI green with the new job; release tagged; docs published.

## Phase 6 - downstream handoff

Tracked here, executed in `ace-router-debug`.

- [ ] Refresh `vendor/rusty-bacnet` to the released commit via the
      documented explicit operation; `PROVENANCE.json` updated.
- [ ] Add `bacnet-mstp-codec` and `bacnet-mstp-core` to the firmware build
      graph; remove any application adapter the requests said would become
      redundant.
- [ ] Node F receive-only run with the codec and P4 observer; then MAC 30
      active alongside the C masters at 2, 3, 10, and 20.
- [ ] Capture the acceptance evidence the requests list (token fairness,
      Who-Is/I-Am and property interoperability, UART and CRC error deltas,
      DE timing on the logic analyzer, overload isolation, rollback) under
      `docs/plans/evidence/embedded-mstp/`.

Acceptance: ACE sign-off recorded with the firmware commit and the vendored
release.

## Proposed PR queue

| # | Title | Phase | Upstreamable | Size |
|---|---|---|---|---|
| 1 | ci: seed embedded cross-compile with `bacnet-types` | 0 | yes | S |
| 2 | encoding: no_std + alloc behind a default std feature | 1 | yes | S |
| 3 | services: no_std + alloc behind a default std feature | 1 | yes | M |
| 4 | mstp-codec: new no_std frame codec crate with golden and property tests | 2 | yes | L |
| 5 | transport: adapt mstp_frame to the codec crate | 2 | yes | S |
| 6 | mstp-core: new no_std master core with simulated-time tests | 3 | yes | L |
| 7 | transport: drive MstpTransport from the core, counters, DE/RE actions | 4 | yes | L |
| 8 | docs and release: embedded section, changelog, fork release | 5 | docs yes | S |

PRs 2 and 3 are independent of 4. PR 5 depends on 4. PR 6 depends on 4. PR 7
depends on 6 and 5. PR 1 lands first so every later PR is checked on the
embedded targets.

## Validation gates

### G1 - per-PR host

```
cargo fmt --all -- --check
cargo clippy --workspace --exclude rusty-bacnet --all-targets --locked
bash .github/scripts/check-file-size.sh
cargo test -p <crate> --locked
```

### G2 - embedded cross-compile

```
cargo check -p bacnet-types -p bacnet-encoding -p bacnet-services \
  -p bacnet-mstp-codec -p bacnet-mstp-core \
  --no-default-features --target thumbv7em-none-eabi --locked
cargo check ... --features alloc --target riscv32imac-unknown-none-elf --locked
cargo check -p bacnet-mstp-core --no-default-features --features heapless \
  --target thumbv7em-none-eabi --locked
```

### G3 - dependency tree

```
bash .github/scripts/check-embedded-deps.sh
```

Inspects both packages and activated features. It fails if `tokio`, `socket2`,
`libc`, `tokio-serial`, `gpiocdev`, or `tracing` appears as a normal dependency,
or if a dependency's `std` feature is activated for either new crate under the
feature combination being checked. Real bare-target compilation remains the
authoritative proof; tree inspection prevents accidental host-only baggage.

### G4 - golden and differential

The codec differential test and the ported 47 codec tests; the ported master
tests under simulated time.

### G5 - property and fuzz

`proptest` suites in both new crates run in CI; `cargo fuzz run mstp_decode`
for at least ten minutes locally before PR 4 merges, with the corpus checked
in.

### G6 - host regression

Workspace suite with the CI feature set; installed-wheel Python suite
including `test_mstp_compat.py`; the W5 virtual router container scenario.

### G7 - hardware

ACE's node F and P4 observer runs per Phase 6, outside this repository.

## Risks

- `tracing` in `no_std` builds. Mitigation: gate every call site behind
  `std` and provide no-op macros; do not take on `defmt` in this plan.
- The 700-line cap versus a state machine that is 665 lines before it gains
  configuration, queues, and counters. Mitigation: the module split in Phase
  3 is part of the design, not cleanup.
- Behavior drift between host and core when `T_frame_abort` becomes exact.
  Mitigation: the host adapter keeps its documented approximation by
  feeding `on_silence` from read timeouts; the differential tests assert
  the host path, not the exact path.
- Upstream shape disagreement after code exists. Mitigation: Phase 0 files
  the issues first; the new crates are additive and survive a rejection as
  fork-only crates.
- Upstream drift: `bacnet-transport/src/mstp` has moved 1590 commits
  upstream. Mitigation: PRs 5 and 7 are the only ones touching it; keep
  them small and rebase them last.
- Public host inspection currently exposes `Arc<Mutex<MasterNode>>` through
  `node_state()`. Mitigation: preserve that signature through a deprecated
  wrapper and add a separate snapshot API; M8 applies to the portable core,
  not to this host compatibility layer.
- `heapless` adds a dependency to a crate meant to be minimal. Mitigation:
  it is a feature, and the slice-based `QueueStorage` impl needs no crate.

## Decisions

Ratified 2026-10-10.

1. **Crate names:** `bacnet-mstp-codec` and `bacnet-mstp-core`, matching the
   existing `bacnet-` prefix and the request's two boundaries.
2. **Frame ownership:** the codec crate owns a `FrameHeader` value type plus
   borrowed frame data. The host `MstpFrame` with `Bytes` stays in the
   `bacnet-transport` adapter.
3. **COBS extended frames:** explicit rejection and a reserved `cobs` feature
   only. No implementation in this plan.
4. **Version line:** this work lands on the fork's 0.11.x line. A 0.12
   reconciliation with upstream is not a prerequisite; if the sc-ip-router
   plan's E3 later chooses one, Phase 0 re-pins and the new crates carry over
   unchanged because they are additive.

Still open:

- Whether `bacnet-objects` and `bacnet-network` should also gain `no_std`.
  Out of scope here; the requests do not ask for them.

## Out of scope

- A `no_std` server, client, or router.
- ESP-IDF or `esp-hal` platform code; the platform owns UART, DMA, GPIO, and
  timers per the request.
- COBS and CRC-32K extended frame support beyond explicit rejection.
- Changes to B/IP, BBMD, or SC code paths in `bacnet-transport`.
- Any change to the Python MS/TP constructor signatures.
