#!/bin/sh
set -eu

if ! command -v podman >/dev/null 2>&1; then
    echo "podman is required to run the same-port discovery fixture" >&2
    exit 127
fi

if ! podman compose version >/dev/null 2>&1; then
    echo "podman compose (or podman-compose) is required" >&2
    exit 127
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../../../.." && pwd)
compose_file="$script_dir/compose.yml"
project_name="rusty-bacnet-python-discovery"
image_name="localhost/rusty-bacnet-python-discovery:dev"

cleanup() {
    podman compose --project-name "$project_name" --file "$compose_file" down \
        --volumes --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

podman build --tag "$image_name" --file "$script_dir/Dockerfile" "$repo_root"
podman build --tag localhost/rusty-bacnet-bacpypes3-bbmd:dev \
    --file "$script_dir/Dockerfile.bacpypes3" "$repo_root"

podman compose --project-name "$project_name" --file "$compose_file" up \
    --abort-on-container-exit --exit-code-from client
