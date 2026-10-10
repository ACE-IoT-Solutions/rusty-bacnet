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

## Current acceptance matrix

| Evidence | State |
|---|---|
| Codec regression floor | 47/47 passing |
| Master/host-adapter regression floor | 62/62 passing |
| Bare Cortex-M `bacnet-types` | Phase 0 gate added |
| Bare RISC-V `bacnet-types` | Phase 0 gate added |
| `bacnet-encoding` and `bacnet-services` bare builds | Phase 1 |
| Standalone codec dependency gate | Phase 2 |
| Portable core dependency gate | Phase 3 |
| Installed-wheel and W5 regression | Phase 4/5 |
| Node F and P4 hardware evidence | Phase 6, downstream |
