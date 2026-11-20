#!/usr/bin/env bash
set -euo pipefail

if ! command -v ccc >/dev/null 2>&1; then
  echo "error: ccc is not installed" >&2
  exit 127
fi

common_dir=$(git rev-parse --path-format=absolute --git-common-dir)
if [[ $(basename "$common_dir") != ".git" ]]; then
  echo "error: cannot resolve the canonical checkout from $common_dir" >&2
  exit 1
fi
canonical_root=$(dirname "$common_dir")

if [[ ! -f "$canonical_root/.cocoindex_code/settings.yml" ]]; then
  echo "error: initialize the shared index once in $canonical_root with 'ccc init && ccc index'" >&2
  exit 1
fi

if [[ $# -eq 0 ]]; then
  echo "usage: scripts/ccc-shared.sh <ccc arguments...>" >&2
  echo "example: scripts/ccc-shared.sh search 'routed property requests'" >&2
  exit 2
fi

cd "$canonical_root"
exec ccc "$@"
