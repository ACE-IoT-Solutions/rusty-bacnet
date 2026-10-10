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
    -e normal,features)

  if grep -Eq '(bacnet-types|bacnet-encoding|bacnet-services|bytes) feature "std"|(^|[[:space:]])tracing (v|feature )' <<<"$tree"; then
    echo "error: ${package} activates a host-only dependency feature for ${target}" >&2
    printf '%s\n' "$tree" >&2
    exit 1
  fi
done

for features in none alloc heapless alloc,heapless std std,heapless; do
  feature_args=(--no-default-features)
  if [[ "$features" != none ]]; then
    feature_args+=(--features "$features")
  fi

  tree=$(cargo tree \
    -p bacnet-mstp-codec \
    "${feature_args[@]}" \
    --target "$target" \
    --locked \
    -e normal)

  if grep -Eq '(^|[[:space:]├└]── )(tokio|socket2|libc|tokio-serial|tracing) v' <<<"$tree"; then
    echo "error: bacnet-mstp-codec activates a host-only dependency for ${target} (${features})" >&2
    printf '%s\n' "$tree" >&2
    exit 1
  fi
done

echo "OK: embedded dependency trees are free of host-only dependencies for ${target}."
