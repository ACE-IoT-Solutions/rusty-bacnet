#!/bin/sh
set -eu

if ! command -v podman >/dev/null 2>&1; then
    echo "podman is required to run the W3 BBMD policy fixture" >&2
    exit 127
fi
if ! podman compose version >/dev/null 2>&1; then
    echo "podman compose (or podman-compose) is required" >&2
    exit 127
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../../../.." && pwd)
compose_file="$script_dir/compose.w3-bbmd-policy.yml"
project_name="rusty-bacnet-w3-bbmd-$$"
wheel_image="localhost/rusty-bacnet-python-discovery:dev"
bacpypes_image="localhost/rusty-bacnet-bacpypes3-bbmd:dev"

compose() {
    podman compose --project-name "$project_name" --file "$compose_file" "$@"
}

cleanup() {
    compose down --volumes --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

container_id() {
    printf '%s_%s_1\n' "$project_name" "$1"
}

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

assert_log_count() {
    container=$1
    marker=$2
    expected=$3
    actual=$(podman logs "$container" 2>&1 | grep -F -c "$marker" || true)
    if [ "$actual" -ne "$expected" ]; then
        podman logs "$container" >&2 || true
        echo "marker count mismatch: marker='$marker' expected=$expected actual=$actual" >&2
        return 1
    fi
}

signal_command() {
    container=$1
    command=$2
    podman exec "$container" touch "/tmp/$command"
}

if [ "${W3_SKIP_BUILD:-0}" != "1" ]; then
    podman build --tag "$wheel_image" --file "$script_dir/Dockerfile" "$repo_root"
    podman build --tag "$bacpypes_image" --file "$script_dir/Dockerfile.bacpypes3" "$repo_root"
fi

compose config >/dev/null
compose up --detach --no-deps bbmd_a bbmd_b
bbmd_a=$(container_id bbmd_a)
bbmd_b=$(container_id bbmd_b)
wait_for_log "$bbmd_a" "BBMD_READY side=A" 20
wait_for_log "$bbmd_b" "BBMD_READY side=B" 20

compose up --detach --no-deps fd_a fd_b
fd_a=$(container_id fd_a)
fd_b=$(container_id fd_b)
wait_for_log "$fd_a" "FD_REGISTERED side=A" 20
wait_for_log "$fd_b" "FD_REGISTERED side=B" 20
wait_for_log "$bbmd_a" "BBMD_FDT_ACTIVE side=A" 10
wait_for_log "$bbmd_b" "BBMD_FDT_ACTIVE side=B" 10

signal_command "$fd_a" send-allow-a
wait_for_log "$fd_a" "FD_SENT side=A token=ALLOW_A" 5
wait_for_log "$fd_b" "FD_RECEIVED side=B token=ALLOW_A" 10

signal_command "$fd_b" send-allow-b
wait_for_log "$fd_b" "FD_SENT side=B token=ALLOW_B" 5
wait_for_log "$fd_a" "FD_RECEIVED side=A token=ALLOW_B" 10

signal_command "$fd_a" send-cut
wait_for_log "$fd_a" "FD_SENT side=A token=CUT" 5
wait_for_log "$fd_b" "FD_RECEIVED side=B token=CUT" 10

signal_command "$fd_a" send-drop
wait_for_log "$fd_a" "FD_SENT side=A token=DROP" 5
sleep 2
assert_log_count "$fd_b" "FD_RECEIVED side=B token=DROP" 0

compose up --detach --no-deps reject_probe
reject_probe=$(container_id reject_probe)
wait_for_log "$reject_probe" "FD_REJECTED side=REJECT code=0x0030" 15
reject_exit=$(podman wait "$reject_probe")
if [ "$reject_exit" -ne 0 ]; then
    podman logs "$reject_probe" >&2 || true
    exit "$reject_exit"
fi

sleep 1
assert_log_count "$fd_b" "FD_RECEIVED side=B token=ALLOW_A" 1
assert_log_count "$fd_a" "FD_RECEIVED side=A token=ALLOW_B" 1
assert_log_count "$fd_b" "FD_RECEIVED side=B token=CUT" 1
assert_log_count "$fd_a" "FD_RECEIVED side=A token=ALLOW_A" 0
assert_log_count "$fd_b" "FD_RECEIVED side=B token=ALLOW_B" 0
assert_log_count "$fd_a" "FD_RECEIVED side=A token=CUT" 0
assert_log_count "$fd_a" "FD_RECEIVED side=A token=DROP" 0

compose stop --timeout 1 fd_b
signal_command "$bbmd_b" w3-arm-expiry
wait_for_log "$bbmd_b" "BBMD_FDT_EXPIRY_ARMED side=B" 5
sleep 34
signal_command "$bbmd_b" w3-check-expiry
wait_for_log "$bbmd_b" "BBMD_FDT_EXPIRED side=B" 45

signal_command "$bbmd_a" w3-report
signal_command "$bbmd_b" w3-report
wait_for_log "$bbmd_a" "BBMD_COUNTERS side=A" 10
wait_for_log "$bbmd_b" "BBMD_COUNTERS side=B" 10

for live_container in "$bbmd_a" "$bbmd_b" "$fd_a"; do
    if [ "$(podman inspect --format '{{.State.Running}}' "$live_container")" != "true" ]; then
        podman logs "$live_container" >&2 || true
        echo "required fixture service exited before acceptance: $live_container" >&2
        exit 1
    fi
done

podman logs "$bbmd_a"
podman logs "$bbmd_b"
podman logs "$fd_a"

compose down --volumes --remove-orphans
trap - EXIT INT TERM
for removed_container in "$bbmd_a" "$bbmd_b" "$fd_a" "$fd_b" "$reject_probe"; do
    if podman container exists "$removed_container"; then
        echo "fixture container remained after cleanup: $removed_container" >&2
        exit 1
    fi
done
if podman network exists "${project_name}_w3_carrier"; then
    echo "fixture network remained after cleanup: ${project_name}_w3_carrier" >&2
    exit 1
fi
printf '%s\n' "W3_CLEANUP_OK"
printf '%s\n' "W3_BBMD_POLICY_ACCEPTANCE_PASS"
