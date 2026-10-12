# Embedded MS/TP work plan

Status: decided 2026-10-11 to adopt upstream's merged codec, core, and no_std
crates (#1612, #1616, #1619) as the base and park the fork's own copies for
later re-integration. Phase 7 (adopt upstream) is the implementation team's
next work; Phase 6 hardware open
Created: 2026-10-10
Target line: `fix/sc-router-review-closeout`; embedded MS/TP released at
`55233111` (`v0.11.7`). The firmware still vendors the `9f5e019`/0.11.4
starting point; rebase onto `main` once PR #4 merges.
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
  (Superseded 2026-10-11 by F1: the maintainer has stated pre-1.0 APIs may
  break and wants one implementation; see the alignment review below.)
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
| Incremental decode (K2) | `StreamDecoder` with fixed internal state and a caller-owned data buffer; `push(byte) -> Event` and `push_slice`; synchronization noise is distinct from a classified malformed-frame event | Byte-at-a-time matches UART ISR delivery; slice form matches DMA chunks; the distinction is required by the Clause 9 receive FSM |
| Codec errors | `FrameError` enum with no payload strings; `From<FrameError> for bacnet_types::Error` under `alloc` | Removes `format!` from the hot path and keeps host error text identical through the adapter |
| COBS (K4) | `FrameType::is_cobs()` and `FrameError::UnsupportedCobsFrame` with exhaustive tests for every type in 32..=127; do not publish a Cargo `cobs` feature until support exists | Explicit rejection today without making `--all-features` intentionally uncompilable |
| Core API (M1-M3) | `MasterCore<Q: QueueStorage>` with `on_octet(byte, now)`, `on_receive_error(kind, now)`, `on_silence(now)`, `on_deadline(now)`, request-bound `complete_request(id, decision, now)`, generation-bound `on_transmit_complete(id, now)` / `on_transmit_aborted(id, reason, now)`, `enqueue_npdu(dest, npdu, priority, now)`, `poll_action()`, `next_deadline()`; `Action::{SetDirection(Direction), Transmit{id, bytes}, DeliverNpdu{request_id, ..}, Idle}` | `Transmit` bytes remain borrowed only until the next poll. Completion means the final stop bit has physically left the UART, not merely that DMA or an OS buffer accepted it. A separate bounded delivery ring cannot consume the control capacity needed to restore receive direction or pass the token. |
| Clock (M2) | `Instant` and `Duration` from a caller-supplied `Monotonic` trait with microsecond resolution; the core stores absolute deadlines | Avoids `std::time`, lets ESP timers and simulated time share one path |
| Queues (M4) | `QueueStorage` trait with two impls: `HeaplessQueues<const U: usize, const B: usize, const N: usize>` and `AllocQueues` (`alloc` feature); per-queue depth, high-water, drop counters by reason `{Full, Oversize, Expired, Starved}`; broadcast-to-unicast ratio and max-age in config | Bounded by construction on embedded, unchanged depth on host |
| Configuration (M5) | `MstpCoreConfig` with every Clause 9 timer, `N_retry_token`, `N_min_octets`, `N_poll`, `Max_Master`, `Max_Info_Frames`, multiple-frame ratio, baud; `validate()` enforces standard bounds and returns a typed reason | Replaces the `const` timers at `mstp/mod.rs:45-64` |
| Counters (M6) | `MstpCounters` plain struct of `u32` fields, `Copy`, read by `counters()`; DE assertion duration accumulated from `SetDirection` pairs using the injected clock | No atomics or locks inside the core (M8) |
| Host adapter (M7, K5) | `bacnet-transport::mstp` becomes a Tokio driver over `MasterCore<AllocQueues>`: embedded/wire-timestamped input feeds `on_octet`; the host keeps its chunk-tolerant assembler and feeds a validated-frame ingress; `sleep_until(next_deadline)` feeds `on_deadline`; actions map to serial writes and the existing DE/RE modes | Keeps `MstpTransport<S>` and the 62 host tests as the regression floor without pretending USB chunk-arrival times are wire inter-octet timestamps |
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

- [x] `MstpCoreConfig` with every Clause 9 parameter and `validate()`; port
      the existing `MasterNode::new` checks and add the standard ranges.
- [x] `Monotonic` trait, `Instant`, `Duration`; a `SimClock` test helper.
- [x] `MasterCore` state machine ported from both `mstp/mod.rs` and the
      timer/driver transitions in `mstp/port.rs`, keeping the
      `MasterState` names and Clause 9.5.6 transitions; `on_octet` runs the
      codec `StreamDecoder` and models `T_frame_abort` exactly.
- [x] `QueueStorage` with heapless and alloc impls; class, priority,
      starvation limit, expiry, and typed rejection reasons.
- [x] Separate fixed control and delivery action rings and `poll_action`;
      `SetDirection` emitted before every transmit and receive direction is
      restored only after the matching generation's last stop bit has left
      the UART. Add explicit transmit abort/reset handling.
- [x] Request-bound application completion: `DeliverNpdu` carries an opaque
      `RequestId`; data, ReplyPostponed, cancellation, expiry, duplicates, and
      stale replies cannot complete a newer request from the same source.
- [x] Receive-error input and classified parser outcomes distinguish ordinary
      synchronization noise from invalid frames, CRC failures, unsupported
      COBS framing, and caller-buffer exhaustion.
- [x] `MstpCounters` with the fields the request names.
- [x] Split modules so no file exceeds the cap: `config`, `clock`, `queue`,
      `state`, `actions`, `counters`.
- [x] Tests: deterministic simulated-time tests for every master state and
      every deadline edge, ported from `clause956_tests.rs` and `tests.rs`;
      queue overflow proves token passing continues; `proptest` on random
      event interleavings asserting no panic, no allocation after `new`
      (checked with a counting allocator under `alloc`), monotonic event
      timestamps, state-appropriate causal deadlines, stale timer rejection,
      and bounded progress after a due deadline. Deadlines are not globally
      monotone because a receive event can install an earlier reply deadline.
- [x] Dependency-tree script passes; `#![forbid(unsafe_code)]`; no `std`,
      no `tokio`, no locks.

Acceptance: core cross-compiles for both embedded targets with `heapless`
only; all simulated-time tests green; counters observable in every test.

## Phase 4 - Tokio adapter over the core (M7, K5)

Crates: `bacnet-transport`.

- [x] Rewrite `mstp/port.rs` to drive `MasterCore<AllocQueues>`: the host's
      bounded, chunk-tolerant assembler feeds validated frames (wire-timestamped
      platforms use `on_octet`), `tokio::time::sleep_until` feeds
      `on_deadline`, and `poll_action` maps to drained serial writes and to the
      existing DE/RE modes in `mstp_serial.rs`.
- [x] Keep `MstpTransport<S>`, `MstpConfig` (now converting into
      `MstpCoreConfig`), `SerialPort`, `LoopbackSerial`, `NoSerial`,
      the existing `node_state() -> Option<&Arc<Mutex<MasterNode>>>` contract,
      and `AnyTransport::Mstp` source-compatible. Retain `MasterNode` as a
      deprecated adapter wrapper and add a separately named snapshot API;
      changing `node_state()` itself would break current callers and tests.
- [x] Expose `counters()` on `MstpTransport`. `RouterPortCounters` remains a
      transport-neutral forwarding snapshot; coupling its synchronous atomic
      surface to the adapter's async MS/TP mutex did not fit that abstraction.
- [x] Python: `mstp_py.rs` and the client, server, and router lifecycles
      compile unchanged; add `counters` to the Python MS/TP surface only if
      the stub change is trivial, otherwise defer.
- [x] Delete the codec differential scaffolding after cutover. The host runtime
      no longer invokes the old state-machine methods; their deprecated public
      fields and methods remain solely because deleting them conflicts with
      this plan's explicit source-compatibility requirement for `MasterNode`.

Tests: the 62 host master/adapter tests pass unchanged; `test_mstp_compat.py` passes
on an installed wheel; a new adapter test proves a `SetDirection` pair
brackets every transmit with the GPIO mode.

Acceptance: host behavior unchanged; workspace suite and Python suite green.

## Phase 5 - feature matrix, docs, and release

- [x] `embedded-check` covers both targets and the feature matrix: `no_std`,
      `no_std + alloc`, `no_std + heapless`, `std`, and the Tokio adapter.
- [x] `.github/scripts/check-embedded-deps.sh` asserts the dependency tree
      for both new crates and runs in the fast gate.
- [x] `docs/rust-api.md`: new "Embedded MS/TP" section with a bare-metal
      codec example, a core driving loop example, and the counters table.
- [x] `CHANGELOG.md` entries; version bump per the Phase 0 decision; fork
      release through the existing `ace-pypi` path so the Python wheel picks
      up the adapter.
- [x] Update the two upstream issues with the PR links.

Acceptance: CI green with the new job; release tagged; docs published.

Release evidence (2026-10-10): tag `v0.11.7` points to `55233111`. GitHub
Actions run [38075269965](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/38075269965)
passed the complete validation matrix, built CPython 3.13 manylinux wheels for
x86_64 and aarch64, macOS wheels for x86_64 and arm64, a Windows x86_64 wheel,
and the repaired source distribution, then published and verified all six
artifacts through ACE PyPI. A separate clean Python 3.13 environment installed
`ace-rusty-bacnet==0.11.7` from the private index and imported
`rusty_bacnet` successfully.

Follow-up release evidence (2026-10-10): tag `v0.11.8` points to `aa72df40`.
GitHub Actions run
[38081262565](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/38081262565)
passed and published a repaired source distribution plus CPython 3.13
manylinux x86_64/aarch64, macOS x86_64/arm64, and Windows x86_64 wheels. A
clean macOS arm64 environment installed `ace-rusty-bacnet==0.11.8` from ACE
PyPI and imported `rusty_bacnet` successfully.

Upstream handoff evidence (2026-10-10): issue
[#1604](https://github.com/jscott3201/rusty-bacnet/issues/1604) links the
focused allocator-free codec
[PR #1606](https://github.com/jscott3201/rusty-bacnet/pull/1606), and issue
[#1605](https://github.com/jscott3201/rusty-bacnet/issues/1605) links the
stacked portable master-core
[PR #1607](https://github.com/jscott3201/rusty-bacnet/pull/1607). The latter
is intentionally stacked on the codec PR and narrows to its core commit after
the codec merges.

## Phase 6 - downstream handoff

Tracked here, executed in `ace-router-debug`.

- [x] Refresh `vendor/rusty-bacnet` to the released commit via the
      documented explicit operation; `PROVENANCE.json` updated.
- [x] Add `bacnet-mstp-codec` and `bacnet-mstp-core` to the firmware build
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

Downstream implementation evidence (2026-10-10):

- `ace-router-debug/firmware/rust-espidf/vendor/rusty-bacnet` is an exact,
  checksum-verified export of release `0.11.7` at commit `55233111`; its
  refresh and inventory-check scripts reject both modified and extra files.
- The node F workspace builds separate passive and `active-mstp` ESP32-S3
  release images from the vendored codec and core. Their recorded ELF SHA-256
  values are `e7eb2ec5ff1c1bbc83a59f7692f077403d2a9421d9ca4a6d844ba07f96280446`
  and `295c40a91ef4de18fe60697e3a54582dae5caf172039533dad96929ce168015c`.
- Seven tests pass in both feature modes, including physical transmit
  completion before receive restore, fail-closed recovery after write or drain
  failure, Who-Is/I-Am, ReadProperty, and WriteProperty/read-back for the
  deterministic node-F model. The target dependency tree contains the codec,
  core, alloc-backed encoding/services, and ESP-IDF platform crates but no
  Tokio, `socket2`, or `bacnet-transport` host runtime.
- The evidence evaluator is fail-closed: missing captures or measurements are
  `invalid`, completed failed assertions are `fail`, and only complete
  passive/active manifests can become `pass`. The downstream Python suites
  pass 29 repository tests and four dashboard tests.
- Hardware acceptance is not yet claimed. No complete passing node F/P4
  capture has been recorded, and the downstream workspace has no Git metadata
  from which to record the required firmware commit. These keep the two
  hardware checklist items and Phase 6 acceptance open.

First hardware attempt (2026-10-10): the passive ELF was converted to merged
factory image `b52684b20d39a37958144228bfd1c89f9ff0e7f9d4a5cbd9e46cf64903e875da`
and flashed to node F after staging the verified rollback image on the ARM HIL
host. The Rust image booted and held `DE=0`, but eleven consecutive reports
over 55 seconds showed `rx_bytes=0` while observer C continued receiving valid
MS/TP traffic. The run was cancelled, the active image was not flashed, and the
C factory image was restored; its health endpoint and MS/TP counters recovered.
The incomplete capture is intentionally `invalid`, not a pass. The next passive
attempt must resolve the ESP-IDF UART receive path and add independent DE and
heap measurements before the hardware checklist can advance. A follow-up
passive diagnostic used ESP-IDF native RS-485 half-duplex mode with RTS on GPIO
21, but it also received zero UART bytes over 15 seconds. That diagnostic was
reverted; the canonical images rebuilt to their recorded hashes, and the known
C image was flash-verified again. After 78 seconds F was healthy with
`rx_bytes=1928` and `tx_bytes=15879`, while observer C was healthy and its last
valid frame came from MAC 30. The next attempt therefore requires physical or
logic-analyzer inspection of F's receive path and the current lab wiring rather
than another blind firmware flash.

Subsequent passive diagnostics (2026-10-10) isolated the zero-byte result
further. Run `20261010T193126Z-7c97b89d` used a QIO image incompatible with
the node's 16 MB flash/OPI PSRAM configuration and boot-looped. Run
`20261010T193322Z-3eb60e56` restored DIO and the aligned memory configuration,
but a normal UART timeout was logged continuously and starved the observation
loop. Run `20261010T193533Z-9b7a618c` fixed that timeout classification and
booted the passive factory image
`865b89c04eef72119246560c50e68126f3f222525170a59f8f4fa8ea56d30dd0`.
It held `DE=0` with stable heap (`free_heap=8716772`, minimum `8716244`) but
still reported zero bytes and zero read calls. Over the same interval observer
C advanced by 1044 bytes and 115 valid, CRC-valid frames with no header CRC,
data CRC, or UART errors, proving that the live bus was active. The known C
factory image was restored and both F and C recovered. These runs rule out
flash/PSRAM mismatch, timeout log starvation, timeout misclassification, and
observer inactivity; they do not satisfy passive acceptance.

The raw-UART passive image replaces the high-level `esp-idf-hal` UART driver
with a narrow owner-backed adapter using the same ESP-IDF UART1 initialization
and read calls as the known-working C node. Both downstream feature-mode test
suites pass seven tests, all 35 evaluator/dashboard Python tests pass, and the
vendored inventory is exact. The target build produced passive ELF
`6e0c3d2b206c47c0e865c5e77142a3648735f294ca22db36f53300bc8137b12c`
and merged DIO factory image
`8f784ce3c5c284532655649c0d64a8e61c94a5db9341378df29f8c057d323519`.
The generated configuration retains 16 MB flash, OPI PSRAM, 1 kHz FreeRTOS
ticks, and the UART ISR-in-IRAM setting.

Bounded diagnostic `20261011T012252Z-raw-uart` flashed that exact factory
image after verifying the USB serial device and rollback hash. In 40 seconds,
F advanced from 976 to 7784 UART bytes and from 122 to 973 valid frames, with
zero read, malformed-frame, or codec errors. Every report held `DE=0`, free
heap stayed at 8716772 bytes, and minimum free heap stayed at 8716244 bytes.
Peer E concurrently advanced by 8182 receive bytes, independently proving the
bus remained live. The guarded runner restored the known C image, verified the
flash hash, restarted the dashboard, and F resumed token participation. This
isolates the prior zero-byte failure to the extra UART mode/interrupt setup in
the high-level HAL path and validates the raw ESP-IDF receive path.

This diagnostic does not close passive acceptance: the inventory still pointed
to observer C's former management address, so its counters were not captured,
and no independent logic-analyzer DE trace was recorded. C was subsequently
located at `192.168.9.110`. The active image remains unflashed until a repeat
run includes both artifacts and earns a fail-closed passive `pass` verdict.

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
  Mitigation: wire-timestamped platforms use `on_octet`; the host adapter
  retains its documented chunk-tolerant assembly and uses the core's
  validated-frame ingress. Differential tests cover the host path separately
  from exact embedded inter-octet tests.
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
3. **COBS extended frames:** originally explicit rejection only. Superseded
   2026-10-11: the maintainer stated that MS/TP routing requires Extended
   frame types 32 and 33, and the fork implemented COBS/CRC-32K and the
   extended codec, core, and router profiles (`b8f0bb0e`, `724b40ca`,
   `da7a4eda`). Explicit rejection remains the behavior of the standard
   profile; see F3 for the upstream sequencing.
4. **Version line:** originally the fork's 0.11.x line. Reopened 2026-10-11:
   the canonical `bacnet-mstp-codec` and `bacnet-mstp-core` now live on
   upstream `dev` at 0.12.0 with review changes the fork lacks. Carrying
   0.11.x copies of crates upstream owns invites permanent divergence.
   Decided 2026-10-11: take the upstream work and move forward. The firmware
   and the fork consume the embedded crates from upstream `dev`; the fork's
   own codec, core, extended-frame, and router work is parked on an archive
   branch. Upstream continues to take what is useful from it, and the fork
   re-integrates later (see Phase 7).

Still open:

- Whether `bacnet-objects` and `bacnet-network` should also gain `no_std`.
  Out of scope here; the requests do not ask for them.

## Review 2026-10-11 - upstream alignment and verification

Independent review of `fix/sc-router-review-closeout` at `da18670` against
the maintainer's responses on
[#1604](https://github.com/jscott3201/rusty-bacnet/issues/1604) and
[#1605](https://github.com/jscott3201/rusty-bacnet/issues/1605) and the two
upstream PRs [#1606](https://github.com/jscott3201/rusty-bacnet/pull/1606)
and [#1607](https://github.com/jscott3201/rusty-bacnet/pull/1607), both still
unreviewed and rebased on upstream `dev` `74fca30` with zero commits behind.

| Check | Result |
|---|---|
| Workspace suite, CI feature set, cleaned crates | 77 suites, 4957 passed, 1 failed (see router plan G2: stale generated conformance doc) |
| `cargo fmt --check`, 700-line gate | clean |
| `check-embedded-deps.sh` on both bare targets | pass |
| Cross-compile for `thumbv7em-none-eabi` and `riscv32imac-unknown-none-elf`: types, codec, codec+heapless, core, core+heapless, router-core, router-adapters, encoding+alloc, services+alloc | all 18 pass |

Local note: a Homebrew `rustc` ahead of rustup on `PATH` makes every bare
target fail with "can't find crate for core" even though
`rust-toolchain.toml` lists the targets. Pin `RUSTC` to the 1.97.1 toolchain
or put `~/.cargo/bin` first. Worth a line in the toolchain docs.

### Where the maintainer's direction and the fork agree

New crates (accepted explicitly); the event contract (`on_octet`,
`on_host_activity`, `on_receive_error`, `on_silence`, `on_deadline`,
absolute deadlines); driver acceptance distinguished from physical completion
(`on_transmit_started`, `on_transmit_complete`, `on_transmit_aborted`);
generation-bound `TransmitId` and `RequestId`; reserved control and reply
capacity in the action scheduler; `Direction` as a core-owned action;
standard-fixed parameters pinned by `MstpCoreConfig::validate()` (`N_poll`,
`N_retry_token`, `N_min_octets`, `T_no_token`, `T_slot`) with ranges only
where the standard permits; hosted adapters kept out of PR #1607; PyO3 kept
in the host layer.

### Findings

- [ ] **F1 - Two implementations of the master state still exist on the host
      path.** The maintainer wrote that pre-1.0 APIs may break, that nothing
      requires retaining `MasterNode` or deprecated wrappers, and that there
      should be one implementation of the protocol behavior. The host adapter
      keeps `#[deprecated] MasterNode` and mirrors core state into it
      (`mstp/core_adapter.rs::sync_legacy`, `mstp/port.rs` `mirror_legacy_*`).
      Remove the legacy struct and the mirroring, migrate the remaining
      callers and tests, and drop the "removals are not allowed" working rule.
- [ ] **F2 - The codec's default profile still depends on `bacnet-types`.**
      The maintainer asked that the strict MS/TP crates not depend on the
      allocation-bearing `bacnet-types` surface. The codec only uses it for
      `From<FrameError> for bacnet_types::Error` under `alloc`, but the Cargo
      dependency is unconditional in both the fork and PR #1606, and
      `bacnet-types` does `extern crate alloc` unconditionally when `std` is
      off. Make the dependency optional and enable it from the `alloc`
      feature, then tighten `check-embedded-deps.sh` to assert its absence in
      the default profile. Apply the same change to PR #1606.
- [ ] **F3 - Sequence the extended-frame work upstream.** Extended types 32
      and 33, COBS, and CRC-32K now exist on the fork but are not in PR #1606
      or #1607. Stack a third upstream PR for the codec and core extended
      profiles once the first two have a review, and say so on #1604. The
      standard-frame checkpoint is what upstream has seen.
- [ ] **F4 - Prove an allocator-free link, not only a check.** The maintainer
      separates an allocator-free smoke image from an integrated firmware
      image with `alloc`. CI runs `cargo check` and the dependency-tree
      script; nothing links a `#![no_std] #![no_main]` binary without a
      global allocator against the codec and core. Add a tiny link-smoke
      binary crate (excluded from default members) built for
      `thumbv7em-none-eabi` in the embedded job.
- [ ] **F5 - Record which tests are standard-backed and which are
      legacy-differential.** The maintainer wants differential checks only
      where the old behavior is known correct, plus independent standard
      vectors. Extended frames now have Annex X.4 fixtures
      (`tests/extended_wire_fixtures.rs`); the standard-frame codec is still
      proved mainly by differential tests against `mstp_frame.rs`. Add an
      evidence table naming the source for each case, and keep the
      consolidated 135-2020 gap (X0 in the router plan) as the bar for any
      conformance claim.
- [ ] **F6 - Ask for the maintainer's regression candidates.** The #1604
      reply mentions "specific source-level timing/address cases" found in
      their first review that should be reproduced before extraction. We do
      not have the list. Ask for it on #1605 and add each as a named test.
- [ ] **F7 - Answer the three alignment questions on the thread.** The
      maintainer asked to settle the storage and transmit contract, the first
      supported role and hardware setup, and which landed changes go upstream
      first. Post one comment: the contract as implemented (events, ids,
      completion, direction ownership, reserved capacity), the first role as
      standard-frame passive observer then MAC 30 master without routing, and
      the proposed order alloc-profile encoding/services, codec, core,
      extended, adapters.
- [ ] **F8 - The `no_std + alloc` encoding/services slice has no upstream
      PR.** The maintainer offered to carry `251e74d2` forward against current
      `dev`. Open that PR, or ask them to, so K1 is not stranded behind the
      MS/TP PRs.
- [ ] **F9 - Pinned constants are still configuration fields.** `n_poll`,
      `n_retry_token`, `n_min_octets`, `t_no_token`, and `t_slot` are public
      fields that `validate()` rejects unless they hold the standard's single
      value. Make them associated constants and remove the fields. Small,
      pre-1.0, and it matches "only parameters the standard permits to vary
      should become tuning knobs".
- [ ] **F10 - BACnet/IP scope ran ahead of upstream agreement.** The
      maintainer proposed a bounded follow-on: BVLC codecs first, then a
      small normal or foreign-device engine. Overnight the fork added
      `bacnet-router-core` and `bacnet-router-adapters` with a bounded B/IP
      UDP adapter, about 7,000 lines, with no upstream issue. Open the issue
      before offering any of it upstream, and label those crates as fork
      provisional in the router plan until then. See the router plan's
      review for the library findings.

Hardware status is unchanged and honestly recorded: two passive attempts, a
raw UART diagnostic that did receive bytes, no passing fail-closed capture,
no logic-analyzer DE trace, active image not flashed. Phase 6 stays open.

## Upstream merge adjustments 2026-10-11

Upstream `dev` moved from `74fca30` to `4aeb3dbb` (20 commits) and merged, in
order, the codec as [#1612](https://github.com/jscott3201/rusty-bacnet/pull/1612)
(supersedes #1606), the maintainer's own `no_std + alloc` build of types,
encoding, and services as [#1616](https://github.com/jscott3201/rusty-bacnet/pull/1616)
(supersedes the fork's `251e74d2`), and the core as
[#1619](https://github.com/jscott3201/rusty-bacnet/pull/1619) (supersedes
#1607). The fork's commits were carried forward with authorship intact; the
maintainer's commits on top are breaking. #1604 is closed; #1605 stays open
for the follow-ups.

### What changed in review (the fork does not have these)

- Codec: `std` feature replaced by `bytes`; the `bacnet-types` dependency
  removed, so the default build has no dependencies and builds on
  `thumbv6m`; decode accepts slave sources 128-254; encode refuses source
  255, data on Token, Poll For Master, Reply To Poll For Master and Reply
  Postponed, and proprietary frames without a vendor id
  (`FrameError::MissingVendorId`, `FrameType::is_proprietary`); the stream
  decoder reports `FrameTooLong` once and resynchronizes instead of wedging;
  `DecodeEvent::Malformed` carries `destination`; `BufferExhausted` reports
  the refused length and free room; Annex G.1 vector added.
- Core: `MstpCoreConfig::reply_transmit_margin` (default 25 ms) and
  `octet_time`; Reply Postponed timing per Clause 9.5.6.9 with new counters
  `late_replies`, `expired_requests`, `cancelled_sends`,
  `discarded_while_answering`; replies accepted from any source; dropped
  candidates filtered by destination; NPDUs over 501 octets refused as
  `Oversize` and never hold the token (a real bug in the original); a
  transmit must be started before it completes; an aborted transmit purges
  its actions; frames received while answering are handled per 9.5.6.2.
- types, encoding, services: upstream wiring is `std = ["alloc", ...]` and
  `alloc`, `tracing` only under `std`, `Error` is `#[non_exhaustive]`,
  `Error::Transport` only with `std`.
- CI: `scripts/ci/check-no-std.sh` runs clippy on the no_std crates for
  `thumbv6m-none-eabi`, `thumbv7em-none-eabihf`, and
  `riscv32imac-unknown-none-elf`; `scripts/ci/check-default-features.sh`
  checks each published crate on Linux, Windows, and macOS.
- Docs: `docs/mstp-qualification.md` and its JSON result template define an
  operator-run bench qualification method and the claim language allowed.

The fork's two crates now differ from upstream by 37 files, about 3,800
lines added and 3,400 removed, and `bacnet-router-core`,
`bacnet-router-adapters`, and the Phase 4 Tokio adapter are written against
the pre-review core API.

### Maintainer follow-up issues that now define the queue

| Issue | Topic | Fork position |
|---|---|---|
| [#1608](https://github.com/jscott3201/rusty-bacnet/issues/1608) | Host MS/TP MAC departs from Clause 9 in six places (U1-U6); fix by cutting the host transport over to codec + core and deleting the old MAC | Replaces our Phase 4 adapter design; the fork's legacy `MasterNode` mirror must not go upstream |
| [#1609](https://github.com/jscott3201/rusty-bacnet/issues/1609) | Codec: skip a dropped frame's body by count; discard noise runs in one event | Not started |
| [#1610](https://github.com/jscott3201/rusty-bacnet/issues/1610) | Codec: fuzz `StreamDecoder` and sinks; build the fuzz crate in CI | Fork has `fuzz/fuzz_targets/mstp_decode.rs`; offer it |
| [#1611](https://github.com/jscott3201/rusty-bacnet/issues/1611) | Codec: extended frames types 32/33, COBS, CRC-32K, required for routing | Fork has X1-X3 implemented; the next PR |
| [#1613](https://github.com/jscott3201/rusty-bacnet/issues/1613) | Run alloc-only tests and MSRV in CI; lift thumbv6m skips | Not started |
| [#1614](https://github.com/jscott3201/rusty-bacnet/issues/1614) | Writer-trait encoders and an allocation-free services subset | Not started; shapes any future allocation-free router application layer |
| [#1615](https://github.com/jscott3201/rusty-bacnet/issues/1615) | Port `bacnet-objects` to no_std + alloc | Not started; was our "still open" question, now upstream-owned |
| [#1617](https://github.com/jscott3201/rusty-bacnet/issues/1617) | Core: `on_invalid_frame` input for host-assembled frames; deliver Test_Response and proprietary frames | Must land before host cutover; fork adapter uses `on_validated_frame` and is affected |
| [#1618](https://github.com/jscott3201/rusty-bacnet/issues/1618) | Core: missed-slot policy, wait-for-silence sends, action-ring contract, construction without a 28 KB stack copy | Maintainer: settle before any firmware relies on the core; gates the active image |

### Disposition of F1-F10

| Finding | Disposition |
|---|---|
| F1 legacy `MasterNode` mirror | Superseded by #1608: upstream will delete the old MAC and cut over. Stop maintaining the mirror on the fork; do not include it in any PR. |
| F2 codec depends on `bacnet-types` | Resolved upstream in #1612. Fork resyncs (Phase 7). |
| F3 sequence extended frames | #1611 is the vehicle. Rebase the fork's X1-X3 onto the merged codec and core and open the PR. |
| F4 allocator-free link proof | Resolved upstream by #1626: CI links the MS/TP and B/IP stacks into no-allocator images with size budgets. |
| F5 standard-backed versus differential tests | Upstream added the Annex G.1 vector for standard frames. Extended fixtures stay the fork's contribution under #1611. |
| F6 maintainer's regression candidates | Answered: #1608 U1-U6. Add them as named tests in the fork's host adapter until the cutover removes it. |
| F7 answer the three alignment questions | Overtaken: the maintainer reviewed the contract directly and encoded the remaining questions in #1617 and #1618. Respond there, not on #1604. |
| F8 alloc-profile slice has no PR | Resolved upstream in #1616 with the maintainer's wiring, which differs from `251e74d2`. Fork adopts upstream's feature wiring in Phase 7. |
| F9 pinned constants as fields | Still true upstream. Raise on #1618 as a small follow-up. |
| F10 router scope ahead of upstream | Still open. #1614 and #1615 show upstream's own direction for the application layer; open the router-boundary issue before any router PR. |

## Phase 7 - adopt the upstream crates and park the fork copies

State on `feature/sc-ip-router-upstream-0.12` (reviewed 2026-10-11): the team
merged upstream `dev` at `bdac34b5`, so the codec and core are upstream's
merged versions with the fork's extended-frame commits rebased on top, and
`bacnet-router-core` and `bacnet-router-adapters` are retained. That satisfies
"take the upstream work and move forward" as upstream-base plus a fork
series rather than as parking; items 1 and 2 below therefore apply to the
pre-merge head `da18670` (archive it) and to any future divergence, not to
the extended series. Items 3 and 4 are done by the merge. Upstream has since
closed #1626 (no-allocator link images in CI, which resolves F4) and #1625
(wait for a quiet line, part of #1618); #1617 and #1618 remain the gate for
the active image.

Decision (2026-10-11): take the upstream work and move forward. Upstream can
keep taking what is useful from the fork's branch; the fork re-integrates its
extended-frame and router work later, on top of whatever upstream ships.

- [ ] Tag the current branch head as `archive/embedded-fork-20261011` and push
      it, so the fork's codec, core, extended-frame, router-core, and
      router-adapter work stays addressable for upstream and for later
      re-integration.
- [ ] Remove the fork's `crates/bacnet-mstp-codec`, `crates/bacnet-mstp-core`,
      `crates/bacnet-router-core`, and `crates/bacnet-router-adapters` from
      the working branch and the workspace members. Do not try to merge them
      with upstream's versions.
- [ ] Bring upstream `dev` at `4aeb3dbb` or later into the fork for the
      embedded crates and the no_std wiring of types, encoding, and services
      (#1616 replaces `251e74d2`). The simplest route is the sc-ip-router
      plan's E3: rebase the fork onto upstream `dev` once PR #4 merges,
      rather than cherry-picking the embedded commits in isolation.
- [ ] Drop the fork's `embedded-check` job and
      `.github/scripts/check-embedded-deps.sh` in favour of upstream's
      `scripts/ci/check-no-std.sh` and `check-default-features.sh`.
- [ ] Firmware: repoint `vendor/rusty-bacnet` at the upstream `dev` commit
      for the embedded crates and record it in `PROVENANCE.json`; adapt the
      ESP-IDF adapter to the merged core API (`Oversize` refusal,
      `reply_transmit_margin`, `octet_time`, start-before-complete transmit,
      action purge on abort, `DecodeEvent::Malformed { destination }`).
      Standard frames only until upstream lands #1611.
- [ ] Do not flash an active image until upstream settles #1617 and #1618,
      or the firmware team documents its own policy for each item.
- [ ] Record in the evidence README which fork evidence (W14, bench, host
      tests) still applies to the upstream crates and which was specific to
      the parked copies.

Acceptance: the fork and the firmware build only upstream-sourced embedded
crates; the archive tag exists; no fork copy of an upstream-owned crate
remains on the working branch.

## Upstream engagement after the decision

No fork PRs are required. The team engages where it is cheap and useful:

- Point #1611 at the archive tag's extended-frame commits as reference
  material; upstream decides what to take.
- Point #1610 at the archive tag's fuzz target.
- Report firmware findings on #1617 and #1618 as they arise from the ESP32
  bench; those two issues gate the active image.
- Raise F9 (pinned constants as fields) and F4 (allocator-free link proof)
  as comments on #1618 and #1613 when convenient.
- Re-open the router boundary conversation upstream only when the fork is
  ready to re-integrate `bacnet-router-core`; until then it stays archived.

The earlier "Revised upstream PR queue" and the F1-F10 dispositions above
are retained as the record of why; they are no longer a work queue.

## Out of scope

- A `no_std` server, client, or router.
- ESP-IDF or `esp-hal` platform code; the platform owns UART, DMA, GPIO, and
  timers per the request.
- COBS and CRC-32K extended frame support beyond explicit rejection.
- Changes to B/IP, BBMD, or SC code paths in `bacnet-transport`.
- Any change to the Python MS/TP constructor signatures.
