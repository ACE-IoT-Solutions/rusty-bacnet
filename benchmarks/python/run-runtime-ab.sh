#!/bin/sh
set -eu

if ! command -v podman >/dev/null 2>&1; then
    echo "podman is required to run the runtime A/B benchmark" >&2
    exit 127
fi
if ! podman compose version >/dev/null 2>&1; then
    echo "podman compose (or podman-compose) is required" >&2
    exit 127
fi

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/../.." && pwd)
source_context=${BACNET_BENCH_SOURCE_CONTEXT:-$repo_root}
compose_file="$script_dir/compose.runtime-ab.yml"
project_name=${BACNET_BENCH_PROJECT_NAME:-rusty-bacnet-runtime-ab}
base_image=${BACNET_BENCH_BASE_IMAGE_NAME:-localhost/rusty-bacnet-sc-runtime:dev}
image_name=${BACNET_BENCH_IMAGE_NAME:-localhost/rusty-bacnet-runtime-ab:dev}
result_dir=${BACNET_BENCH_RESULT_DIR:-"$repo_root/target/runtime-ab"}

if [ ! -f "$source_context/Cargo.toml" ]; then
    echo "BACNET_BENCH_SOURCE_CONTEXT must name a rusty-bacnet source checkout" >&2
    exit 2
fi

source_sha=${BACNET_BENCH_SOURCE_SHA:-$(git -C "$source_context" rev-parse HEAD)}
source_ref=${BACNET_BENCH_SOURCE_REF:-$(git -C "$source_context" symbolic-ref --short -q HEAD || printf 'detached')}
if [ -n "$(git -C "$source_context" status --porcelain --untracked-files=all)" ]; then
    source_dirty=true
else
    source_dirty=false
fi
if [ "$source_dirty" = true ] && [ "${BACNET_BENCH_ALLOW_DIRTY:-0}" != 1 ]; then
    echo "source context is dirty; set BACNET_BENCH_ALLOW_DIRTY=1 for harness validation only" >&2
    exit 2
fi
source_archive_sha256=${BACNET_BENCH_SOURCE_ARCHIVE_SHA256:-$(git -C "$source_context" archive --format=tar HEAD | sha256sum | awk '{print $1}')}

cleanup() {
    podman compose --project-name "$project_name" --file "$compose_file" down \
        --volumes --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

mkdir -p "$result_dir"
export BACNET_BENCH_RESULT_DIR="$result_dir"
export BACNET_BENCH_IMAGE_NAME="$image_name"
export BACNET_BENCH_SOURCE_CONTEXT="$source_context"
export BACNET_BENCH_SOURCE_SHA="$source_sha"
export BACNET_BENCH_SOURCE_REF="$source_ref"
export BACNET_BENCH_SOURCE_DIRTY="$source_dirty"
export BACNET_BENCH_SOURCE_ARCHIVE_SHA256="$source_archive_sha256"

podman build --tag "$base_image" \
    --build-arg RUST_IMAGE="${BACNET_BENCH_RUST_IMAGE:-rust:1.97.1-bookworm}" \
    --build-arg PYTHON_IMAGE="${BACNET_BENCH_PYTHON_IMAGE:-python:3.11-slim-bookworm}" \
    --build-arg SOURCE_SHA="$source_sha" \
    --build-arg SOURCE_REF="$source_ref" \
    --build-arg SOURCE_DIRTY="$source_dirty" \
    --build-arg SOURCE_ARCHIVE_SHA256="$source_archive_sha256" \
    --file "$repo_root/crates/rusty-bacnet/tests/container/Dockerfile.sc-runtime" \
    "$source_context"
podman build --tag "$image_name" \
    --build-arg RUNTIME_BASE_IMAGE="$base_image" \
    --file "$script_dir/Dockerfile.runtime-ab" "$script_dir"

export BACNET_BENCH_BASE_IMAGE_ID
BACNET_BENCH_BASE_IMAGE_ID=$(podman image inspect --format '{{.Id}}' "$base_image")
export BACNET_BENCH_IMAGE_ID
BACNET_BENCH_IMAGE_ID=$(podman image inspect --format '{{.Id}}' "$image_name")

podman compose --project-name "$project_name" --file "$compose_file" config > "$result_dir/compose.resolved.yml"
podman compose --project-name "$project_name" --file "$compose_file" up \
    --abort-on-container-exit --exit-code-from client

test -s "$result_dir/runtime-batch-ab.json"
