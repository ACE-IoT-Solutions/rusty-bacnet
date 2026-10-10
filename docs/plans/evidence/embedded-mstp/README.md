# Embedded MS/TP evidence

This directory records reproducible evidence for
`docs/plans/embedded-mstp-workplan.md`. Hardware evidence remains downstream in
`ace-router-debug`; this file covers the repository-side baseline and gates.

## 2026-10-10 baseline

Source revision: `9f5e019fd3a080059ad0b4b893db189a59aa9c32` (`v0.11.4`).
The downstream provenance file names the same commit and release.

Host commands and results:

```text
cargo test -p bacnet-transport --lib --locked --offline mstp_frame::tests::
47 passed; 0 failed; 0 ignored; 484 filtered out

cargo test -p bacnet-transport --lib --locked --offline mstp::
62 passed; 0 failed; 0 ignored; 469 filtered out

cargo fmt --all -- --check
passed

bash .github/scripts/check-file-size.sh
passed: all tracked Rust files within the 700 effective-LOC cap

git diff --check
passed
```

The 62-test MS/TP floor comprises 41 general state-machine tests, 12 Clause
9.5.6 tests, three timing tests, two reply-ownership tests, and four host
assembly/abort tests in `mstp/port.rs`.

The host commands above used Homebrew Cargo/Rust 1.98.1 because those binaries
precede rustup on this workstation's `PATH`. Bare-target evidence explicitly
uses the repository's pinned Rust 1.97.1 compiler:

```text
RUSTC=/Users/acedrew/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc \
  rustup run 1.97.1 cargo check -p bacnet-types --no-default-features \
  --target thumbv7em-none-eabi --locked --offline

RUSTC=/Users/acedrew/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc \
  rustup run 1.97.1 cargo check -p bacnet-types --no-default-features \
  --target riscv32imac-unknown-none-elf --locked --offline
```

Before Phase 0, both commands failed in `thiserror` because the workspace
enabled `thiserror/default -> thiserror/std`. Phase 0 disables that workspace
default feature and makes both commands CI gates. The GitHub runner does not
need the local `RUSTC` workaround because the rustup toolchain action controls
its `PATH`.

## Phase 1 local acceptance

Both alloc-backed codec crates passed the following checks on 2026-10-10. The
bare-target commands used the pinned Rust 1.97.1 `rustc`; doctests also used
the matching `rustdoc`.

```text
cargo test -p bacnet-encoding -p bacnet-services --lib --locked --offline
280 encoding tests and 531 service tests passed

cargo test -p bacnet-encoding -p bacnet-services --lib \
  --no-default-features --features alloc --locked --offline
280 encoding tests and 531 service tests passed

cargo test -p bacnet-encoding -p bacnet-services --doc \
  --no-default-features --features alloc --locked --offline
one doctest per crate passed

cargo check -p bacnet-encoding -p bacnet-services \
  --no-default-features --features alloc \
  --target thumbv7em-none-eabi --locked --offline
passed

cargo check -p bacnet-encoding -p bacnet-services \
  --no-default-features --features alloc \
  --target riscv32imac-unknown-none-elf --locked --offline
passed

cargo check -p bacnet-objects -p bacnet-network --locked --offline
passed

bash .github/scripts/check-embedded-deps.sh <target>
passed for both bare targets; no std or tracing activation

cargo test --workspace --exclude rusty-bacnet --locked --offline
passed after rerunning one timing-sensitive runtime overflow test in isolation

cargo fmt --all -- --check
bash .github/scripts/check-file-size.sh
bash .github/scripts/check-no-secrets.sh
actionlint .github/workflows/ci.yml
git diff --check
all passed
```

The source diff changes no public item signatures. The only collection-type
change is the private segmentation buffer from `HashMap` to `BTreeMap`.

## Phase 2 local acceptance

The standalone MS/TP codec and compatibility adapter passed the following
checks on 2026-10-10 with pinned Rust and Rustdoc 1.97.1:

```text
cargo test -p bacnet-mstp-codec --all-features --locked --offline
19 codec unit/property tests passed; doctests passed

cargo test -p bacnet-transport --lib --locked --offline mstp_frame
52 tests passed, including the 47-test legacy floor and four differential tests

cargo test -p bacnet-transport --lib --locked --offline mstp
115 tests passed, including the 62-test master/adapter floor

cargo check -p bacnet-mstp-codec --no-default-features --locked --offline \
  --target <target>
passed for thumbv7em-none-eabi and riscv32imac-unknown-none-elf

bash .github/scripts/check-embedded-deps.sh <target>
passed for both bare targets and the no-feature, alloc, heapless,
alloc+heapless, std, and std+heapless feature combinations
```

The compatibility review additionally linked a real bare-metal executable
using `Frame::encode_into` without a global allocator. Differential coverage
preserves legacy wire bytes, exact errors and offsets, stream discard choices,
single-byte CRC behavior, and error precedence. Property tests cover
truncation, corruption, oversized wire lengths, preamble resynchronization,
byte-at-a-time input, and sink exhaustion/recovery.

## Phase 3 portable-core acceptance (2026-10-10)

The first portable-core implementation passed its host feature-matrix checks
with the bounded heapless backend enabled:

```text
cargo check -p bacnet-mstp-core --all-features --locked
passed

cargo test -p bacnet-mstp-core --all-features --locked --offline
20 unit tests, 11 Clause 9 regression tests, and 11 public-core/property tests
passed; 0 failed; doctests passed

RUSTC=/Users/acedrew/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc \
  rustup run 1.97.1 cargo check -p bacnet-mstp-core \
  --no-default-features --target thumbv7em-none-eabi --locked --offline
passed

RUSTC=/Users/acedrew/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc \
  rustup run 1.97.1 cargo check -p bacnet-mstp-core \
  --no-default-features --features heapless \
  --target thumbv7em-none-eabi --locked --offline
passed

RUSTC=/Users/acedrew/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc \
  rustup run 1.97.1 cargo check -p bacnet-mstp-core \
  --no-default-features --target riscv32imac-unknown-none-elf --locked --offline
passed

RUSTC=/Users/acedrew/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc \
  rustup run 1.97.1 cargo check -p bacnet-mstp-core \
  --no-default-features --features heapless \
  --target riscv32imac-unknown-none-elf --locked --offline
passed

bash .github/scripts/check-embedded-deps.sh thumbv7em-none-eabi
bash .github/scripts/check-embedded-deps.sh riscv32imac-unknown-none-elf
both passed; embedded dependency trees contain no host-only dependencies
```

The core checks include generation-bound request and transmit completion,
explicit receive-error and malformed-frame handling, strict `T_frame_abort`
and `Nmin_octets` boundaries, station-slot token regeneration, `Tusage_timeout`
polling, failed-successor wrap, broadcast delivery, bounded action priority,
queue overflow/expiry, allocation-free operation after construction, and a
monotone randomized event sequence. These are implementation and regression
evidence; hardware timing and Node F/P4 interoperability remain Phase 6 work.

## Phase 4 host-adapter acceptance (2026-10-10)

The Tokio adapter now drives the portable core and passed the expanded host
regression floor:

```text
cargo test -p bacnet-transport --locked 'mstp::'
69 tests passed; 0 failed

scripts/publish-python-release.sh 0.11.7 --build-only --allow-dirty
repaired sdist built with --locked; CPython 3.13 macOS arm64 wheel built,
installed in an isolated environment, and import-smoked

python -I -m pytest crates/rusty-bacnet/tests/test_mstp_compat.py -q
11 tests and 42 subtests passed against the installed wheel
```

The added adapter regressions cover a scheduling wake at 226 ms while the
250 ms peer-safe reply bound remains intact, host chunk activity across
`Tusage_timeout`, bounded write/drain and direction futures, delayed direction
crossing reply expiry, explicit `TX -> write -> drain -> RX` ordering, and
non-FIFO compatibility-queue mirroring. Shutdown restoration and transient
receive-direction recovery are also covered. The Phase 2 differential
scaffold was removed after the adapter cutover; the 47 original codec cases
remain.

## Current acceptance matrix

| Evidence | State |
|---|---|
| Codec regression floor | 47/47 passing |
| Master/host-adapter regression floor | 62/62 original tests plus seven adapter regressions passing (69 total) |
| Bare Cortex-M `bacnet-types` | `v0.11.5` CI passed |
| Bare RISC-V `bacnet-types` | `v0.11.5` CI passed |
| `bacnet-encoding` and `bacnet-services` bare builds | CI run `38067493332` passed both targets |
| Standalone codec dependency gate | Phase 2 local acceptance passed both targets and all feature combinations; `v0.11.6` CI pending |
| Portable core host tests | 42 tests passed locally; no-default and heapless bare-target checks passed on both targets |
| Portable core dependency gate | `check-embedded-deps.sh` passed for both targets locally; release CI pending |
| Installed-wheel regression | CPython 3.13 wheel build/import and `test_mstp_compat.py` passed locally |
| W5 virtual-router regression | Phase 5 release gate |
| Node F and P4 hardware evidence | Phase 6, downstream |
