#!/usr/bin/env bash
# The MS/TP firmware link test (#1626): build firmware/mstp-link as a
# bare-metal image for each target, with no global allocator, and check its
# section sizes against firmware/mstp-link/size-budget.toml.
#
#   check-firmware-link.sh               for the default targets below
#   check-firmware-link.sh <triple>...   for those targets
#
# check-no-std.sh only runs clippy, which never links. Linking is what shows
# that nothing in the codec or the core needs an allocator, std or a symbol
# only a hosted target provides: the build fails with "no global memory
# allocator found" if anything uses alloc. Nothing runs the image; the
# scenario it links is checked natively by firmware/mstp-link/tests.
#
# Per target: clippy on the image's code, which a host build compiles out;
# the build, with the root Cargo.toml's `firmware` profile (size-optimized,
# LTO, panic=abort); then scripts/ci/firmware_size.py, which prints .text,
# .rodata, .data and .bss against the budget, the largest objects in RAM and
# any core::fmt code, and fails over budget. The targets need only
# `rustup target add`: they link with the rust-lld that rustc ships.
set -euo pipefail

cd "$(dirname "$0")/../.."
default_targets=(thumbv7m-none-eabi thumbv6m-none-eabi riscv32imc-unknown-none-elf)
budget=firmware/mstp-link/size-budget.toml

if [ $# -gt 0 ]; then targets=("$@"); else targets=("${default_targets[@]}"); fi
for triple in "${targets[@]}"; do
  case $triple in -*|'') echo "usage: $0 [<target triple>...]" >&2; exit 2 ;; esac
done

if command -v rustup >/dev/null; then
  installed=$(rustup target list --installed)
  for triple in "${targets[@]}"; do
    grep -qxF "$triple" <<<"$installed" \
      || { echo "error: target $triple is not installed; rustup target add $triple" >&2; exit 1; }
  done
fi

target_dir=$(cargo metadata --format-version 1 --no-deps --locked \
  | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')

status=0
for triple in "${targets[@]}"; do
  echo "==> mstp-link ($triple)"
  cargo clippy -p mstp-link --locked --target "$triple" -- -D warnings
  cargo build -p mstp-link --locked --target "$triple" --profile firmware
  python3 scripts/ci/firmware_size.py "$target_dir/$triple/firmware/mstp-link" "$budget" "$triple" \
    || status=1
done
exit $status
