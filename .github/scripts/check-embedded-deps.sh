#!/usr/bin/env bash

set -euo pipefail

target=${1:?usage: check-embedded-deps.sh <target>}

for package in bacnet-encoding bacnet-services; do
  tree=$(cargo tree \
    -p "$package" \
    --no-default-features \
    --features alloc \
    --target "$target" \
    --locked \
    -e features)

  if grep -Eq '(bacnet-types|bacnet-encoding|bacnet-services|bytes) feature "std"|(^|[[:space:]])tracing (v|feature )' <<<"$tree"; then
    echo "error: ${package} activates a host-only dependency feature for ${target}" >&2
    printf '%s\n' "$tree" >&2
    exit 1
  fi
done

echo "OK: embedded dependency trees are std/tracing-free for ${target}."
