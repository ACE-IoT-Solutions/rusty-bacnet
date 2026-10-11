#!/usr/bin/env bash
# Clippy for the no_std crates on bare-metal targets, denying warnings, once
# per feature set that changes what they compile (#1604).
#
#   check-no-std.sh               for the default targets below
#   check-no-std.sh <triple>...   for those targets; cargo checks them side by
#                                 side, in one run per crate and feature set
#
# A host build can't prove a crate is no_std: the host always has std and an
# allocator, so a stray `std::` path or a dependency that turns on its std
# feature still compiles. A bare-metal target has no std, and no allocator
# unless a crate asks for alloc, so it fails there. The default targets are
# ARMv6-M (no atomic compare-and-swap, so no alloc::sync), ARMv7E-M with an
# FPU, and RISC-V with atomics. Only lib targets build: tests and their
# dev-dependencies need std. Each target needs only `rustup target add`.
#
# To cover another crate or feature set, add a line to `checks`. To leave out
# a combination that legitimately can't build, add a case to `skip_reason`
# with the reason; the run prints it.
set -euo pipefail

# One line each: the crate, then its features as a comma-separated list, or
# `-` for none. Every line builds with --no-default-features.
checks=(
  "bacnet-mstp-codec -"
  "bacnet-mstp-codec alloc"
  "bacnet-mstp-codec heapless"
  "bacnet-mstp-codec bytes,heapless"
  "bacnet-types -"
)
default_targets=(thumbv6m-none-eabi thumbv7em-none-eabihf riscv32imac-unknown-none-elf)

# Prints why <crate> <features> can't build on <target>, or nothing.
skip_reason() {
  local crate=$1 features=$2 target=$3
  case "$target" in thumbv6m-*)
    case ",$features," in *,bytes,*)
      # bytes needs atomic compare-and-swap. Without it in hardware, bytes
      # takes it from portable-atomic, whose backend (a critical section, or
      # assuming a single core) only the firmware can choose.
      echo "bytes needs a portable-atomic backend that only firmware can choose"
      return ;;
    esac
    if [ "$crate" = bacnet-types ]; then
      # It uses alloc::sync::Arc, which needs atomic compare-and-swap.
      echo "bacnet-types uses alloc::sync, which this target lacks"
      return
    fi ;;
  esac
}

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

for check in "${checks[@]}"; do
  read -r crate features <<<"$check"
  feature_args=(--no-default-features)
  [ "$features" = - ] || feature_args+=(--features "$features")
  target_args=()
  built=()
  for triple in "${targets[@]}"; do
    reason=$(skip_reason "$crate" "$features" "$triple")
    if [ -n "$reason" ]; then
      echo "--> skip $crate (features: $features; $triple): $reason"
    else
      target_args+=(--target "$triple")
      built+=("$triple")
    fi
  done
  # Checked before expanding: macOS's bash 3.2 treats an empty array as
  # unset under `set -u`.
  [ ${#built[@]} -gt 0 ] || continue
  echo "==> $crate (features: $features; ${built[*]})"
  cargo clippy -p "$crate" --locked "${feature_args[@]}" "${target_args[@]}" -- -D warnings
done
