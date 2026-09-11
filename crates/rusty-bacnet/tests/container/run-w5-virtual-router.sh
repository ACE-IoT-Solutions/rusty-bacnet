#!/bin/sh
set -eu

if ! command -v podman >/dev/null 2>&1; then
    echo "podman is required to run the W5 virtual-router fixture" >&2
    exit 127
fi
if ! podman compose version >/dev/null 2>&1; then
    echo "podman compose (or podman-compose) is required" >&2
    exit 127
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../../../.." && pwd)
compose_file="$script_dir/compose.w5-virtual-router.yml"
project_name="rusty-bacnet-w5-router-$$"
wheel_image="localhost/rusty-bacnet-python-discovery:dev"
scanner_image="localhost/rusty-bacnet-bacpypes3-bbmd:dev"
W5_ARTIFACT_DIR=${W5_ARTIFACT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/rusty-bacnet-w5-router.XXXXXX")}

compose() {
    podman compose --project-name "$project_name" --file "$compose_file" "$@"
}

capture_logs() {
    compose logs --no-color host >"$W5_ARTIFACT_DIR/host.log" 2>&1 || true
    compose logs --no-color scanner >"$W5_ARTIFACT_DIR/scanner.log" 2>&1 || true
}

cleanup() {
    capture_logs
    compose down --volumes --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

wait_for_log() {
    container=$1
    marker=$2
    attempts=$3
    count=0
    while [ "$count" -lt "$attempts" ]; do
        logs=$(podman logs "$container" 2>&1 || true)
        if printf '%s\n' "$logs" | grep -F "$marker" >/dev/null; then
            printf '%s\n' "$logs" | grep -F "$marker" | tail -1
            return 0
        fi
        running=$(podman inspect --format '{{.State.Running}}' "$container" 2>/dev/null || true)
        if [ "$running" = "false" ]; then
            printf '%s\n' "$logs" >&2
            echo "container exited before marker: $marker" >&2
            return 1
        fi
        sleep 1
        count=$((count + 1))
    done
    podman logs "$container" >&2 || true
    echo "timed out waiting for marker: $marker" >&2
    return 1
}

container_id() {
    printf '%s_%s_1\n' "$project_name" "$1"
}

mkdir -p "$W5_ARTIFACT_DIR"

if [ "${W5_SKIP_BUILD:-0}" != "1" ]; then
    podman build --tag "$wheel_image" --file "$script_dir/Dockerfile" "$repo_root"
    podman build --tag "$scanner_image" --file "$script_dir/Dockerfile.bacpypes3" "$repo_root"
fi

compose config >/dev/null
compose up --detach --no-deps host
host=$(container_id host)
wait_for_log "$host" W5_ROUTER_READY 30

compose up --detach --no-deps scanner
scanner=$(container_id scanner)
wait_for_log "$scanner" W5_VIRTUAL_ROUTER_ACCEPTANCE_PASS 90
wait_for_log "$host" W5_ROUTER_RESTARTED 10

exit_code=$(podman wait "$scanner")
capture_logs
if [ "$exit_code" -ne 0 ]; then
    podman logs "$scanner" >&2 || true
    exit "$exit_code"
fi

podman logs "$host"
podman logs "$scanner"
printf 'W5_VIRTUAL_ROUTER_ARTIFACT_DIR=%s\n' "$W5_ARTIFACT_DIR"
