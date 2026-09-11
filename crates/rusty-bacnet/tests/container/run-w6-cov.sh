#!/bin/sh
set -eu

if ! command -v podman >/dev/null 2>&1; then
    echo "podman is required to run the W6 COV fixture" >&2
    exit 127
fi
if ! podman compose version >/dev/null 2>&1; then
    echo "podman compose (or podman-compose) is required" >&2
    exit 127
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../../../.." && pwd)
compose_file="$script_dir/compose.w6-cov.yml"
project_name="rusty-bacnet-w6-cov-$$"
wheel_image="localhost/rusty-bacnet-python-discovery:dev"
subscriber_image="localhost/rusty-bacnet-bacpypes3-bbmd:dev"
W6_ARTIFACT_DIR=${W6_ARTIFACT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/rusty-bacnet-w6-cov.XXXXXX")}
export W6_ARTIFACT_DIR

compose() {
    podman compose --project-name "$project_name" --file "$compose_file" "$@"
}

capture_logs() {
    compose logs --no-color server >"$W6_ARTIFACT_DIR/server.log" 2>&1 || true
    compose logs --no-color subscriber >"$W6_ARTIFACT_DIR/subscriber.log" 2>&1 || true
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
        sleep 1
        count=$((count + 1))
    done
    podman logs "$container" >&2 || true
    echo "timed out waiting for marker: $marker" >&2
    return 1
}

mkdir -p "$W6_ARTIFACT_DIR"

if [ "${W6_SKIP_BUILD:-0}" != "1" ]; then
    podman build --tag "$wheel_image" --file "$script_dir/Dockerfile" "$repo_root"
    podman build --tag "$subscriber_image" --file "$script_dir/Dockerfile.bacpypes3" "$repo_root"
fi

compose up --detach --no-deps server
server="${project_name}_server_1"
wait_for_log "$server" W6_COV_SERVER_READY 30

compose up --detach --no-deps subscriber
subscriber="${project_name}_subscriber_1"
wait_for_log "$subscriber" W6_COV_ACCEPTANCE_PASS 300

exit_code=$(podman wait "$subscriber")
capture_logs
if [ "$exit_code" -ne 0 ]; then
    podman logs "$subscriber" >&2 || true
    exit "$exit_code"
fi

podman logs "$subscriber"
printf 'W6_COV_ARTIFACT_DIR=%s\n' "$W6_ARTIFACT_DIR"
