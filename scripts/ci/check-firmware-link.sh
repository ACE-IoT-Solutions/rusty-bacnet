#!/usr/bin/env bash
# The firmware link test (#1626): build firmware/link's images, one per
# stack (mstp: the MS/TP codec and core; bip: the BVLL codec), as bare-metal
# binaries with no global allocator for each target, and check their sizes
# against firmware/link/size-budget.toml.
#
#   check-firmware-link.sh [<triple>...]             check the default targets
#                                                    below, or these
#   check-firmware-link.sh --suggest [<triple>...]   print budget tables for
#                                                    the images as they build
#                                                    now, each size plus 12%
#
# check-no-std.sh only runs clippy, which never links. Linking is what shows
# that nothing in a stack needs an allocator, std or a symbol only a hosted
# target provides: the build fails with "no global memory allocator found"
# if anything uses alloc. Nothing runs the images; the scenarios they link
# are checked natively by firmware/link/tests.
#
# Per target: clippy on the images' code, which a host build compiles out;
# the build, with the root Cargo.toml's `firmware` profile (size-optimized,
# LTO, panic=abort); then, per image, scripts/ci/firmware_size.py, which
# prints the loaded sections, the four main sections and the flash and RAM
# totals against the budget, and any core::fmt code or 64-bit division
# routine, and fails over budget.
# The targets need only `rustup target add`: they link with the rust-lld
# that rustc ships.
set -euo pipefail

cd "$(dirname "$0")/../.."
default_targets=(thumbv7m-none-eabi thumbv6m-none-eabi riscv32imc-unknown-none-elf)
images=(mstp bip)
budget=firmware/link/size-budget.toml

suggest=false
if [ "${1:-}" = --suggest ]; then suggest=true; shift; fi
if [ $# -gt 0 ]; then targets=("$@"); else targets=("${default_targets[@]}"); fi
for triple in "${targets[@]}"; do
  case $triple in -*|'') echo "usage: $0 [--suggest] [<target triple>...]" >&2; exit 2 ;; esac
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
  if "$suggest"; then
    cargo build -q -p firmware-link --locked --target "$triple" --profile firmware
  else
    echo "==> firmware-link ($triple)"
    cargo clippy -p firmware-link --locked --target "$triple" -- -D warnings
    cargo build -p firmware-link --locked --target "$triple" --profile firmware
  fi
  for image in "${images[@]}"; do
    elf=$target_dir/$triple/firmware/$image
    if "$suggest"; then
      python3 scripts/ci/firmware_size.py --suggest "$elf" "$triple.$image"
      echo
    else
      echo "--> $image"
      python3 scripts/ci/firmware_size.py "$elf" "$budget" "$triple.$image" || status=1
    fi
  done
done
exit $status
