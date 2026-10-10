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

for package in bacnet-mstp-codec bacnet-mstp-core; do
  for features in none alloc heapless alloc,heapless std std,heapless; do
    feature_args=(--no-default-features)
    if [[ "$features" != none ]]; then
      feature_args+=(--features "$features")
    fi

    tree=$(cargo tree \
      -p "$package" \
      "${feature_args[@]}" \
      --target "$target" \
      --locked \
      -e normal,features \
      -f '{p} [{f}]')

    if grep -Eq '(^|[[:space:]├└]── )(tokio|socket2|libc|tokio-serial|gpiocdev|tracing) v' <<<"$tree"; then
      echo "error: ${package} activates a host-only dependency for ${target} (${features})" >&2
      printf '%s\n' "$tree" >&2
      exit 1
    fi

    root_features=${tree%%$'\n'*}
    root_features=${root_features##*[}
    root_features=${root_features%]}
    expected_std=false
    if [[ ",$features," == *,std,* ]]; then
      expected_std=true
    fi
    has_std=false
    if [[ ",$root_features," == *,std,* ]]; then
      has_std=true
    fi
    if [[ "$has_std" != "$expected_std" ]]; then
      echo "error: ${package} has an unexpected root std feature for ${target} (${features})" >&2
      printf '%s\n' "$tree" >&2
      exit 1
    fi

    # New crate dependencies must remain no_std, including when the selected
    # root feature is deliberately named `std` (which only opts into alloc and
    # optional bytes in the codec).
    case "$package" in
      bacnet-mstp-codec)
        dependency_tree=$(grep -E '^(bacnet-types|bytes) v[^[]* \[([^]]*,)?std(,[^]]*)?\]' <<<"$tree" || true)
        ;;
      bacnet-mstp-core)
        dependency_tree=$(grep -E '^(bacnet-types|bacnet-mstp-codec|bytes) v[^[]* \[([^]]*,)?std(,[^]]*)?\]' <<<"$tree" || true)
        ;;
    esac
    if [[ -n "$dependency_tree" ]]; then
      echo "error: ${package} activates a dependency std feature for ${target} (${features})" >&2
      printf '%s\n' "$dependency_tree" >&2
      printf '%s\n' "$tree" >&2
      exit 1
    fi
  done
done

echo "OK: embedded dependency trees are free of host-only dependencies for ${target}."
