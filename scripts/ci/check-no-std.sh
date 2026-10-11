#!/usr/bin/env bash
# Clippy for the no_std crates on bare-metal targets, denying warnings, once
# per feature set that changes what they compile, and a check of each build's
# dependency graph (#1604, #1605, #1613).
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
# On ARMv6-M, bytes takes compare-and-swap from portable-atomic, whose
# backend only the final firmware may choose. So a build there with bytes in
# its graph brings in `harness` below (firmware/nostd-harness), which plays
# the firmware: on that target alone it turns on bytes' extra-platforms
# feature and portable-atomic's critical-section backend. Cargo unifies
# features across all the targets of one run, so that build runs on its own,
# and bytes on the other targets builds as their firmware would build it.
# Clippy doesn't link, so nothing implements the critical section.
#
# The graph check reads `cargo tree` for each crate, feature set and target,
# the harness included where it joins. No package in it may have a `std`
# feature on, and none may be one of `host_only` below. It names the package
# when std leaks in (through a workspace entry with default features, say),
# and it catches a leak that still compiles, such as tracing on a target with
# atomics.
#
# To cover another crate or feature set, add a line to `checks`.
set -euo pipefail

# One line each: the crate, then its features as a comma-separated list, or
# `-` for none. Every line builds with --no-default-features.
checks=(
  "bacnet-mstp-codec -"
  "bacnet-mstp-codec alloc"
  "bacnet-mstp-codec heapless"
  "bacnet-mstp-codec bytes,heapless"
  "bacnet-mstp-core -"
  "bacnet-mstp-core alloc"
  "bacnet-mstp-core heapless"
  "bacnet-mstp-core alloc,heapless"
  "bacnet-bip-codec -"
  "bacnet-bip-codec alloc"
  "bacnet-bip-codec heapless"
  "bacnet-bip-codec bytes,heapless"
  "bacnet-types alloc"
  "bacnet-types serde"
  "bacnet-encoding alloc"
  "bacnet-services alloc"
)
default_targets=(thumbv6m-none-eabi thumbv7em-none-eabihf riscv32imac-unknown-none-elf)
# Crates that need std (or, for tracing, atomics the no_std crates don't
# assume). None may appear in a no_std build's graph.
host_only=(tracing tracing-core tokio socket2)
# The firmware stand-in for ARMv6-M builds that use bytes; see above.
harness=nostd-harness

# One line per package of the graph for <cargo tree args...>: `name vX.Y.Z
# [feature,...]`. Proc macros run on the host, so they're left out.
graph() {
  cargo tree --locked "$@" -e normal,no-proc-macro --prefix none --format '{p} [{f}]'
}

# Whether <crate>'s build on <target>, with <feature args...>, brings in the
# harness: on ARMv6-M, when bytes is in its graph. It runs as an `if`
# condition, where `set -e` is off, so a failed cargo tree exits explicitly.
uses_harness() {
  local crate=$1 triple=$2 tree; shift 2
  case $triple in thumbv6m-*) ;; *) return 1 ;; esac
  tree=$(graph -p "$crate" "$@" --target "$triple") || exit 1
  grep -q '^bytes v' <<<"$tree"
}

# Fails if the graph of <packages...> on <target> turns on a std feature or
# pulls in a host-only crate. The arguments after <target> go to cargo tree.
check_graph() {
  local triple=$1; shift
  local tree bad host_re
  host_re=$(IFS='|'; echo "${host_only[*]}")
  tree=$(graph "$@" --target "$triple")
  bad=$(grep -E "\[([^]]*,)?std(,[^]]*)?\]|^($host_re) v" <<<"$tree" || true)
  if [ -n "$bad" ]; then
    echo "error: the $crate graph for $triple ($*) has std in it:" >&2
    sort -u <<<"$bad" >&2
    return 1
  fi
}

# Checks the graph, then runs clippy, for $crate with $feature_args on each
# <target>, in one cargo run; with --harness first, with the harness too.
lint() {
  local packages=(-p "$crate") target_args=() triple with=
  if [ "$1" = --harness ]; then
    packages+=(-p "$harness")
    with="; with $harness"
    shift
  fi
  echo "==> $crate (features: $features; $*$with)"
  for triple in "$@"; do
    check_graph "$triple" "${packages[@]}" "${feature_args[@]}"
    target_args+=(--target "$triple")
  done
  cargo clippy "${packages[@]}" --locked "${feature_args[@]}" "${target_args[@]}" -- -D warnings
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
  together=()
  for triple in "${targets[@]}"; do
    if uses_harness "$crate" "$triple" "${feature_args[@]}"; then
      lint --harness "$triple"
    else
      together+=("$triple")
    fi
  done
  # Checked before expanding: macOS's bash 3.2 treats an empty array as
  # unset under `set -u`.
  if [ ${#together[@]} -gt 0 ]; then lint "${together[@]}"; fi
done
