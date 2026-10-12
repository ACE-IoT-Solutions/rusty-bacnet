#!/usr/bin/env bash
# Runs the firmware link test's images on emulated boards (docs/ci.md,
# "Emulation (manual)"). A manual check, run at intervals: nothing in CI
# calls it but .github/workflows/emulation.yml, which runs only when
# someone starts it.
#
#   emulate-firmware.sh [--native] [--timeout <seconds>] [<board>...]
#
# For each board (all four unless some are named), this builds the mstp,
# bip and apdu images for the board's target with the `emulate` feature and
# the board's, with the pinned toolchain, on the host: clippy first, as
# check-firmware-link.sh does, then the `emulate` profile, which is the link
# test's `firmware` profile under its own name. Then scripts/emu/run-qemu.sh
# runs each image on the board's QEMU machine, in the Docker image that
# scripts/emu/Dockerfile pins, built on first use and tagged with the
# Dockerfile's hash; with --native, on the QEMU on PATH instead (Homebrew's
# `qemu`, say), whose version may differ. Each image checks its scenario's
# summary against the one its host test asserts, reports its stack's
# high-water mark, and exits through semihosting. The script exits non-zero
# if anything fails to build or any image fails or runs past the timeout
# (default 30 s per image).
#
# The boards, their targets and QEMU machines (firmware/link/boards/ holds
# their memory layouts):
#
#   microbit        thumbv6m-none-eabi           Cortex-M0, nRF51822
#   lm3s6965evb     thumbv7m-none-eabi           Cortex-M3, LM3S6965
#   netduinoplus2   thumbv7em-none-eabihf        Cortex-M4F, STM32F405
#   sifive_e        riscv32imc-unknown-none-elf  SiFive E31 (RV32IMAC)
set -euo pipefail

cd "$(dirname "$0")/../.."

# board, target, QEMU system (qemu-system-<it>), and the board's cargo feature.
all_boards=(
  "microbit thumbv6m-none-eabi arm board-microbit"
  "lm3s6965evb thumbv7m-none-eabi arm board-lm3s6965evb"
  "netduinoplus2 thumbv7em-none-eabihf arm board-netduinoplus2"
  "sifive_e riscv32imc-unknown-none-elf riscv32 board-sifive-e"
)
# One cargo run per feature set, as in check-firmware-link.sh: mstp and bip
# without the crate's default `alloc` feature, then apdu with it.
builds=("--no-default-features" "--lib --bin apdu")
images=(mstp bip apdu)

usage() {
  echo "usage: $0 [--native] [--timeout <seconds>] [<board>...]" >&2
  echo "boards: microbit lm3s6965evb netduinoplus2 sifive_e" >&2
  exit 2
}

native=false
timeout=30
wanted=()
while [ $# -gt 0 ]; do
  case $1 in
    --native) native=true ;;
    --timeout)
      [ $# -ge 2 ] || usage
      timeout=$2
      shift
      ;;
    -*) usage ;;
    *) wanted+=("$1") ;;
  esac
  shift
done
case $timeout in '' | *[!0-9]*) usage ;; esac

boards=()
if [ ${#wanted[@]} -eq 0 ]; then
  boards=("${all_boards[@]}")
else
  for name in "${wanted[@]}"; do
    found=
    for board in "${all_boards[@]}"; do
      [ "${board%% *}" = "$name" ] && found=$board
    done
    [ -n "$found" ] || { echo "error: no board named $name" >&2; usage; }
    boards+=("$found")
  done
fi

# Fail early on what the run needs: the targets, and Docker or QEMU.
if command -v rustup >/dev/null; then
  installed=$(rustup target list --installed)
  for board in "${boards[@]}"; do
    read -r _ triple _ _ <<<"$board"
    grep -qxF "$triple" <<<"$installed" \
      || { echo "error: target $triple is not installed; rustup target add $triple" >&2; exit 1; }
  done
fi
if "$native"; then
  for board in "${boards[@]}"; do
    read -r _ _ qemu _ <<<"$board"
    command -v "qemu-system-$qemu" >/dev/null \
      || { echo "error: qemu-system-$qemu is not on PATH (brew install qemu, or drop --native)" >&2; exit 1; }
  done
else
  docker info >/dev/null 2>&1 \
    || { echo "error: Docker isn't running; start it, or use --native with a local QEMU" >&2; exit 1; }
fi

target_dir=$(cargo metadata --format-version 1 --no-deps --locked \
  | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')

jobs=()
for board in "${boards[@]}"; do
  read -r machine triple qemu feature <<<"$board"
  for build in "${builds[@]}"; do
    read -ra args <<<"$build"
    echo "==> firmware-link for $machine ($triple; --features $feature $build)"
    cargo clippy -p firmware-link --locked --target "$triple" --features "$feature" "${args[@]}" -- -D warnings
    cargo build -p firmware-link --locked --target "$triple" --profile emulate --features "$feature" "${args[@]}"
  done
  for image in "${images[@]}"; do
    jobs+=("$image:$triple:$qemu:$machine:$triple/emulate/$image")
  done
done

echo
if "$native"; then
  echo "==> QEMU on the host"
  bash scripts/emu/run-qemu.sh "$timeout" "$target_dir" "${jobs[@]}"
  exit
fi

# The image's tag is the Dockerfile's hash, so an edit to it builds a new
# image instead of reusing a stale one.
if command -v sha256sum >/dev/null; then
  hash=$(sha256sum scripts/emu/Dockerfile)
else
  hash=$(shasum -a 256 scripts/emu/Dockerfile)
fi
tag=rusty-bacnet-emu:${hash:0:12}
if ! docker image inspect "$tag" >/dev/null 2>&1; then
  echo "==> building $tag from scripts/emu/Dockerfile (once; under a minute)"
  docker build --quiet --tag "$tag" - <scripts/emu/Dockerfile
fi
echo "==> QEMU in $tag"
# Read-only mounts and no network: the container only runs QEMU on the
# images and prints what they report.
docker run --rm --network none \
  --volume "$target_dir:/images:ro" \
  --volume "$PWD/scripts/emu:/emu:ro" \
  "$tag" bash /emu/run-qemu.sh "$timeout" /images "${jobs[@]}"
