#!/usr/bin/env bash
# Checks, builds and runs the codecs' cargo-fuzz targets (#1610) with the
# nightly toolchain that .github/ci-pins.env pins as RUST_FUZZ_NIGHTLY.
# .github/workflows/fuzz.yml runs it, and it runs the same way locally (see
# docs/ci.md, "Fuzzing"):
#
#   fuzz.sh list                     one "<crate> <target>" line per target
#   fuzz.sh matrix                   the same as a JSON array, for fuzz.yml
#   fuzz.sh check                    each fuzz crate's lockfile, rustfmt, clippy
#   fuzz.sh build [<target>]         build every target, or one
#   fuzz.sh run <target> <seconds> [<libFuzzer flag>...]
#                                    fuzz one target for that many seconds
#   fuzz.sh tmin <target> [<input>...]
#                                    minimize failing inputs, by default each
#                                    crash or timeout the target has saved
#   fuzz.sh lock                     update each lockfile's own codec entry,
#                                    after a workspace version bump
#
# The fuzz crates are crates/*/fuzz, each a workspace of its own with its own
# Cargo.lock; every build first checks that lockfile with `cargo fetch
# --locked`. With FUZZ_WORK set, corpora go in $FUZZ_WORK/corpus/<target> and
# failing inputs in $FUZZ_WORK/artifacts/<target>; without it, in the fuzz
# crate's corpus/ and artifacts/, cargo-fuzz's defaults, which git ignores.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
pins=$root/.github/ci-pins.env

fail() { echo "error: fuzz.sh: $*" >&2; exit 1; }
# The comment above, as the usage message.
usage() { awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0" >&2; exit 2; }

[ "$#" -gt 0 ] || usage
command=$1; shift

nightly=$(sed -n 's/^RUST_FUZZ_NIGHTLY=//p' "$pins")
[[ $nightly =~ ^nightly-[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] \
  || fail "no single RUST_FUZZ_NIGHTLY=nightly-YYYY-MM-DD in $pins"

# The crates with a fuzz crate, by directory name.
crates=()
for manifest in "$root"/crates/*/fuzz/Cargo.toml; do
  [ -f "$manifest" ] || continue
  dir=${manifest%/fuzz/Cargo.toml}
  crates+=("${dir##*/}")
done
[ ${#crates[@]} -gt 0 ] || fail "no fuzz crates in crates/*/fuzz"

# targets CRATE: the [[bin]] names in CRATE's fuzz manifest.
targets() {
  awk '/^\[\[bin\]\]/ { bin = 1; next }
       /^\[/ { bin = 0 }
       bin && /^name *= *"/ { sub(/^name *= *"/, ""); sub(/".*$/, ""); print }' \
    "$root/crates/$1/fuzz/Cargo.toml"
}

# crate_of TARGET: the crate whose fuzz crate has TARGET.
crate_of() {
  local crate
  for crate in "${crates[@]}"; do
    if targets "$crate" | grep -xF "$1" >/dev/null; then echo "$crate"; return; fi
  done
  fail "no fuzz target $1; fuzz.sh list names them"
}

# cargo with the pinned nightly, which must be installed.
nightly_cargo() {
  rustup toolchain list | grep "^$nightly-" >/dev/null \
    || fail "$nightly is not installed: rustup toolchain install $nightly --profile minimal"
  cargo "+$nightly" "$@"
}

# locked CRATE: fail unless CRATE's fuzz lockfile is current.
locked() {
  nightly_cargo fetch --locked --quiet --manifest-path "$root/crates/$1/fuzz/Cargo.toml" \
    || fail "crates/$1/fuzz/Cargo.lock is out of date; after a version bump, run: bash scripts/ci/fuzz.sh lock"
}

# work KIND TARGET: the corpus or artifact directory for TARGET under
# FUZZ_WORK, created, or nothing without FUZZ_WORK.
work() {
  [ -n "${FUZZ_WORK:-}" ] || return 0
  mkdir -p "$FUZZ_WORK/$1/$2"
  (cd "$FUZZ_WORK/$1/$2" && pwd)
}

case $command in
  list)
    for crate in "${crates[@]}"; do
      targets "$crate" | sed "s/^/$crate /"
    done ;;
  matrix)
    entries=()
    for crate in "${crates[@]}"; do
      for target in $(targets "$crate"); do
        [[ $crate$target =~ ^[a-z0-9_-]+$ ]] || fail "unexpected characters in $crate $target"
        entries+=("{\"crate\":\"$crate\",\"target\":\"$target\"}")
      done
    done
    (IFS=,; echo "[${entries[*]}]") ;;
  check)
    for crate in "${crates[@]}"; do
      manifest=$root/crates/$crate/fuzz/Cargo.toml
      echo "==> $crate"
      locked "$crate"
      nightly_cargo fmt --manifest-path "$manifest" --check
      nightly_cargo clippy --locked --manifest-path "$manifest" --all-targets -- -D warnings
    done ;;
  build)
    [ "$#" -le 1 ] || usage
    if [ "$#" -eq 1 ]; then
      crate=$(crate_of "$1")
      locked "$crate"
      (cd "$root/crates/$crate" && nightly_cargo fuzz build "$1")
    else
      for crate in "${crates[@]}"; do
        echo "==> $crate"
        locked "$crate"
        (cd "$root/crates/$crate" && nightly_cargo fuzz build)
      done
    fi ;;
  run)
    [ "$#" -ge 2 ] || usage
    target=$1 seconds=$2; shift 2
    [[ $seconds =~ ^[1-9][0-9]*$ ]] || fail "seconds must be a positive whole number, not $seconds"
    crate=$(crate_of "$target")
    locked "$crate"
    corpus=$(work corpus "$target")
    artifacts=$(work artifacts "$target")
    # A positional corpus directory replaces cargo-fuzz's default one; a
    # later -artifact_prefix overrides the one cargo-fuzz passes. A single
    # input that runs 10 seconds is a hang.
    (cd "$root/crates/$crate" && nightly_cargo fuzz run "$target" ${corpus:+"$corpus"} -- \
      -max_total_time="$seconds" -timeout=10 -print_final_stats=1 \
      ${artifacts:+"-artifact_prefix=$artifacts/"} "$@") ;;
  tmin)
    [ "$#" -ge 1 ] || usage
    target=$1; shift
    crate=$(crate_of "$target")
    artifacts=$(work artifacts "$target")
    dir=${artifacts:-$root/crates/$crate/fuzz/artifacts/$target}
    inputs=("$@")
    if [ ${#inputs[@]} -eq 0 ]; then
      for input in "$dir"/crash-* "$dir"/timeout-*; do
        [ -f "$input" ] && inputs+=("$input")
      done
    fi
    [ ${#inputs[@]} -gt 0 ] || fail "no failing input for $target in $dir"
    locked "$crate"
    status=0
    for input in "${inputs[@]}"; do
      input=$(cd "$(dirname "$input")" && pwd)/$(basename "$input")
      echo "==> minimizing $input"
      (cd "$root/crates/$crate" && nightly_cargo fuzz tmin "$target" "$input" -- -timeout=10 \
        ${artifacts:+"-artifact_prefix=$artifacts/"}) || status=1
    done
    exit "$status" ;;
  lock)
    # Any cargo will do, so this needs no nightly.
    [ "$#" -eq 0 ] || usage
    for crate in "${crates[@]}"; do
      cargo update --manifest-path "$root/crates/$crate/fuzz/Cargo.toml" -p "$crate"
    done ;;
  *) usage ;;
esac
