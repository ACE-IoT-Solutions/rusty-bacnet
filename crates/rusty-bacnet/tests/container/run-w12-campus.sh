#!/bin/sh
set -eu

command -v podman >/dev/null 2>&1 || { echo "podman is required" >&2; exit 127; }
podman compose version >/dev/null 2>&1 || { echo "podman compose is required" >&2; exit 127; }

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../../../.." && pwd)
compose_file="$script_dir/compose.w12-campus.yml"
project_name="rusty-bacnet-w12-campus-$$"
base_image="localhost/rusty-bacnet-python-discovery:dev"
campus_image="localhost/rusty-bacnet-w12-campus:dev"
W12_ARTIFACT_DIR=${W12_ARTIFACT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/rusty-bacnet-w12-campus.XXXXXX")}
export W12_ARTIFACT_DIR

compose() {
    podman compose --project-name "$project_name" --file "$compose_file" "$@"
}
container_id() { printf '%s_%s_1\n' "$project_name" "$1"; }
capture_logs() {
    for service in campus_router remote_host client bbmd_a bbmd_b; do
        compose logs --no-color "$service" >"$W12_ARTIFACT_DIR/$service.log" 2>&1 || true
    done
}
cleanup() {
    capture_logs
    compose down --volumes --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

wait_for_log() {
    container=$1 marker=$2 attempts=$3 count=0
    while [ "$count" -lt "$attempts" ]; do
        logs=$(podman logs "$container" 2>&1 || true)
        if printf '%s\n' "$logs" | grep -F "$marker" >/dev/null; then
            printf '%s\n' "$logs" | grep -F "$marker" | tail -1
            return 0
        fi
        sleep 1
        count=$((count + 1))
    done
    podman logs "$container" >&2 || true
    echo "timed out waiting for marker: $marker" >&2
    return 1
}

mkdir -p "$W12_ARTIFACT_DIR"
if [ "${W12_SKIP_BUILD:-0}" != "1" ]; then
    podman build --tag "$base_image" --file "$script_dir/Dockerfile" "$repo_root"
    podman build --tag "$campus_image" --file "$script_dir/Dockerfile.w12-campus" "$repo_root"
fi
compose config >/dev/null

compose up --detach --no-deps campus_router remote_host
wait_for_log "$(container_id remote_host)" W12_REMOTE_READY 30
compose up --detach --no-deps client
wait_for_log "$(container_id client)" W12_ISOLATED_DISCOVERY_NEGATIVE_PASS 20

compose up --detach --no-deps bbmd_a bbmd_b
wait_for_log "$(container_id bbmd_a)" W12_BBMD_READY 30
wait_for_log "$(container_id bbmd_b)" W12_BBMD_READY 30
touch "$W12_ARTIFACT_DIR/bbmd.ready"

wait_for_log "$(container_id client)" W12_ACCEPTANCE_PASS 90
wait_for_log "$(container_id bbmd_a)" W12_BBMD_SOURCE_PASS 10
wait_for_log "$(container_id bbmd_b)" W12_BBMD_SOURCE_PASS 10

client_exit=$(podman wait "$(container_id client)")
capture_logs
if [ "$client_exit" -ne 0 ]; then
    podman logs "$(container_id client)" >&2 || true
    exit "$client_exit"
fi

podman logs "$(container_id remote_host)"
podman logs "$(container_id bbmd_a)"
podman logs "$(container_id bbmd_b)"
podman logs "$(container_id client)"
printf 'W12_CAMPUS_ARTIFACT_DIR=%s\n' "$W12_ARTIFACT_DIR"
