#!/bin/sh
set -eu

if ! command -v podman >/dev/null 2>&1; then
    echo "podman is required to run the W2 foreign-device lifecycle fixture" >&2
    exit 127
fi
if ! podman compose version >/dev/null 2>&1; then
    echo "podman compose (or podman-compose) is required" >&2
    exit 127
fi

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/../../../.." && pwd)
compose_file="$script_dir/compose.w2-foreign-lifecycle.yml"
project_name="rusty-bacnet-w2-foreign-$$"
wheel_image="localhost/rusty-bacnet-python-discovery:dev"
bacpypes_image="localhost/rusty-bacnet-bacpypes3-bbmd:dev"
W2_ARTIFACT_DIR=${W2_ARTIFACT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/rusty-bacnet-w2-foreign.XXXXXX")}
export W2_ARTIFACT_DIR

compose() {
    podman compose --project-name "$project_name" --file "$compose_file" "$@"
}

container_id() {
    printf '%s_%s_1\n' "$project_name" "$1"
}

capture_logs() {
    mkdir -p "$W2_ARTIFACT_DIR/logs"
    for service in foreign bbmd broadcaster; do
        compose logs --no-color "$service" >"$W2_ARTIFACT_DIR/logs/$service.log" 2>&1 || true
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

wait_and_record() {
    wait_for_log "$@" >>"$W2_ARTIFACT_DIR/runner-transcript.log"
    tail -1 "$W2_ARTIFACT_DIR/runner-transcript.log"
}

signal_command() {
    container=$1 command=$2
    podman exec "$container" touch "/tmp/$command"
}

if [ "${W2_SKIP_BUILD:-0}" != "1" ]; then
    podman build --tag "$wheel_image" --file "$script_dir/Dockerfile" "$repo_root"
    podman build --tag "$bacpypes_image" --file "$script_dir/Dockerfile.bacpypes3" "$repo_root"
fi

mkdir -p "$W2_ARTIFACT_DIR/logs"
: >"$W2_ARTIFACT_DIR/runner-transcript.log"
compose config >"$W2_ARTIFACT_DIR/compose-config.yml"

# An absent BBMD makes the initial runtime telemetry deterministically pending.
compose up --detach --no-deps foreign
foreign=$(container_id foreign)
wait_and_record "$foreign" "W2_FD_PENDING" 10

compose up --detach --no-deps bbmd
bbmd=$(container_id bbmd)
wait_and_record "$bbmd" "W2_BBMD_READY" 10
wait_and_record "$foreign" "W2_FD_REGISTERED" 12
wait_and_record "$bbmd" "W2_BBMD_TTL_HALF_RENEWAL" 8

compose up --detach --no-deps broadcaster
broadcaster=$(container_id broadcaster)
wait_and_record "$broadcaster" "W2_BROADCAST_SENT" 10
wait_and_record "$foreign" "W2_FD_BROADCAST_RECEIVED" 10
broadcast_exit=$(podman wait "$broadcaster")
if [ "$broadcast_exit" -ne 0 ]; then
    podman logs "$broadcaster" >&2 || true
    exit "$broadcast_exit"
fi

# Loss beyond the advertised TTL must expire health while the attachment stays live.
compose stop --timeout 1 bbmd
wait_and_record "$foreign" "W2_FD_EXPIRED reason=bbmd_outage" 14
compose start bbmd
wait_and_record "$bbmd" "W2_BBMD_READY" 10
wait_and_record "$foreign" "W2_FD_RECOVERED reason=bbmd_restart" 12

# A live BBMD NAK must be distinct from timeout expiry, then recover when cleared.
signal_command "$bbmd" w2-reject
wait_and_record "$foreign" "W2_FD_REJECTED" 8
wait_and_record "$foreign" "W2_FD_EXPIRED reason=rejection" 10
signal_command "$bbmd" w2-allow
wait_and_record "$foreign" "W2_FD_RECOVERED reason=rejection_clear" 10

# Runtime shutdown must cancel renewal; the BBMD then ages out the final FDT row.
signal_command "$foreign" w2-shutdown
wait_and_record "$foreign" "W2_FD_SHUTDOWN_COMPLETE" 10
foreign_exit=$(podman wait "$foreign")
if [ "$foreign_exit" -ne 0 ]; then
    podman logs "$foreign" >&2 || true
    exit "$foreign_exit"
fi
signal_command "$bbmd" w2-arm-shutdown-cleanup
wait_and_record "$bbmd" "W2_BBMD_SHUTDOWN_CLEANUP_ARMED" 5
wait_and_record "$bbmd" "W2_BBMD_FDT_CLEANUP_COMPLETE" 14

signal_command "$bbmd" w2-report
wait_and_record "$bbmd" "unregisters=0 fdt_entries=0" 5
wait_and_record "$foreign" "W2_FOREIGN_LIFECYCLE_ROLE_PASS" 2

capture_logs
python3 -m json.tool "$W2_ARTIFACT_DIR/w2-foreign-transitions.json" >/dev/null
test -s "$W2_ARTIFACT_DIR/w2-bbmd-events.jsonl"

compose down --volumes --remove-orphans
trap - EXIT INT TERM
for removed_container in "$foreign" "$bbmd" "$broadcaster"; do
    if podman container exists "$removed_container"; then
        echo "fixture container remained after cleanup: $removed_container" >&2
        exit 1
    fi
done
if podman network exists "${project_name}_w2_carrier"; then
    echo "fixture network remained after cleanup: ${project_name}_w2_carrier" >&2
    exit 1
fi
printf '%s\n' "W2_CLEANUP_OK" | tee -a "$W2_ARTIFACT_DIR/runner-transcript.log"
printf '%s\n' "W2_FOREIGN_LIFECYCLE_ACCEPTANCE_PASS" | tee -a "$W2_ARTIFACT_DIR/runner-transcript.log"
printf 'W2_FOREIGN_LIFECYCLE_ARTIFACT_DIR=%s\n' "$W2_ARTIFACT_DIR"
