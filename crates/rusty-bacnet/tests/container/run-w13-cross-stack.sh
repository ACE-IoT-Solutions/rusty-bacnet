#!/bin/sh
set -eu

command -v podman >/dev/null 2>&1 || { echo "podman is required" >&2; exit 127; }
podman compose version >/dev/null 2>&1 || { echo "podman compose is required" >&2; exit 127; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required" >&2; exit 127; }

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../../../.." && pwd)
compose_file="$script_dir/compose.w13-cross-stack.yml"
project_name="rusty-bacnet-w13-${$}"
wheel_image="localhost/rusty-bacnet-python-discovery:dev"
bacpypes_image="localhost/rusty-bacnet-bacpypes3-bbmd:dev"
w12_image="localhost/rusty-bacnet-w12-campus:dev"
W13_ARTIFACT_DIR=${W13_ARTIFACT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/rusty-bacnet-w13.XXXXXX")}
export W13_ARTIFACT_DIR

compose() {
    podman compose --project-name "$project_name" --file "$compose_file" "$@"
}

container_id() {
    printf '%s_%s_1\n' "$project_name" "$1"
}

capture_matrix_logs() {
    mkdir -p "$W13_ARTIFACT_DIR/matrix"
    for service in rusty_server bacpypes_server \
        bacpypes_to_bacpypes rusty_to_bacpypes bacpypes_to_rusty rusty_to_rusty; do
        compose logs --no-color "$service" >"$W13_ARTIFACT_DIR/matrix/$service.log" 2>&1 || true
    done
}

cleanup() {
    capture_matrix_logs
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
        if podman container exists "$container" && \
            [ "$(podman inspect --format '{{.State.Running}}' "$container")" != "true" ]; then
            logs=$(podman logs "$container" 2>&1 || true)
            if printf '%s\n' "$logs" | grep -F "$marker" >/dev/null; then
                printf '%s\n' "$logs" | grep -F "$marker" | tail -1
                return 0
            fi
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

run_leg() {
    service=$1
    compose up --detach --no-deps "$service"
    container=$(container_id "$service")
    wait_for_log "$container" "W13_LEG_PASS leg=$service" 90
    status=$(podman wait "$container")
    if [ "$status" -ne 0 ]; then
        podman logs "$container" >&2 || true
        return "$status"
    fi
    printf 'W13_LEG_PASS leg=%s\n' "$service" >>"$W13_ARTIFACT_DIR/runner-transcript.log"
}

run_module() {
    name=$1
    shift
    log="$W13_ARTIFACT_DIR/modules/$name.log"
    if "$@" >"$log" 2>&1; then
        printf 'W13_MODULE_PASS module=%s\n' "$name" >>"$log"
        printf 'W13_MODULE_PASS module=%s\n' "$name" >>"$W13_ARTIFACT_DIR/runner-transcript.log"
        tail -20 "$log"
    else
        status=$?
        tail -80 "$log" >&2 || true
        return "$status"
    fi
}

mkdir -p "$W13_ARTIFACT_DIR/legs" "$W13_ARTIFACT_DIR/commands" "$W13_ARTIFACT_DIR/modules"
: >"$W13_ARTIFACT_DIR/runner-transcript.log"
if [ "${W13_SKIP_BUILD:-0}" != "1" ]; then
    podman build --tag "$wheel_image" --file "$script_dir/Dockerfile" "$repo_root"
    podman build --tag "$bacpypes_image" --file "$script_dir/Dockerfile.bacpypes3" "$repo_root"
    podman build --tag "$w12_image" --file "$script_dir/Dockerfile.w12-campus" "$repo_root"
fi
compose config >/dev/null

compose up --detach --no-deps rusty_server bacpypes_server
wait_for_log "$(container_id rusty_server)" "W13_SERVER_READY stack=rusty" 30
wait_for_log "$(container_id bacpypes_server)" "W13_SERVER_READY stack=bacpypes3" 30

run_leg bacpypes_to_bacpypes
run_leg rusty_to_bacpypes
run_leg bacpypes_to_rusty
run_leg rusty_to_rusty
capture_matrix_logs
compose down --volumes --remove-orphans

run_module w1_discovery env W1_SKIP_BUILD=1 "$script_dir/run-discovery.sh"
run_module w3_bbmd_fdt env W3_SKIP_BUILD=1 "$script_dir/run-w3-bbmd-policy.sh"
mkdir -p "$W13_ARTIFACT_DIR/modules/w5"
run_module w5_routed_sources env W5_SKIP_BUILD=1 \
    W5_ARTIFACT_DIR="$W13_ARTIFACT_DIR/modules/w5" "$script_dir/run-w5-virtual-router.sh"
mkdir -p "$W13_ARTIFACT_DIR/modules/w6"
run_module w6_cov_stream env W6_SKIP_BUILD=1 \
    W6_ARTIFACT_DIR="$W13_ARTIFACT_DIR/modules/w6" "$script_dir/run-w6-cov.sh"
mkdir -p "$W13_ARTIFACT_DIR/modules/w12"
run_module w12_campus env W12_SKIP_BUILD=1 \
    W12_ARTIFACT_DIR="$W13_ARTIFACT_DIR/modules/w12" \
    "$script_dir/run-w12-campus.sh"

for service in rusty_server bacpypes_server \
    bacpypes_to_bacpypes rusty_to_bacpypes bacpypes_to_rusty rusty_to_rusty; do
    if podman container exists "$(container_id "$service")"; then
        echo "W13 container remained after cleanup: $(container_id "$service")" >&2
        exit 1
    fi
done
if podman network exists "${project_name}_matrix"; then
    echo "W13 network remained after cleanup: ${project_name}_matrix" >&2
    exit 1
fi

printf 'W13_CLEANUP_CONTAINERS_ABSENT project=%s\n' "$project_name" \
    >>"$W13_ARTIFACT_DIR/runner-transcript.log"
printf 'W13_CLEANUP_NETWORK_ABSENT network=%s_matrix\n' "$project_name" \
    >>"$W13_ARTIFACT_DIR/runner-transcript.log"
printf 'W13_CLEANUP_OK\n' >>"$W13_ARTIFACT_DIR/runner-transcript.log"
python3 - "$W13_ARTIFACT_DIR" "$project_name" <<'PY'
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
project = sys.argv[2]
(root / "cleanup-attestation.json").write_text(
    json.dumps(
        {
            "schema_version": 1,
            "project": project,
            "scoped_containers_absent": True,
            "scoped_network_absent": True,
            "cleanup_marker": "W13_CLEANUP_OK",
            "transcript": "runner-transcript.log",
        },
        indent=2,
        sort_keys=True,
    )
    + "\n"
)
PY

python3 "$script_dir/w13_compare.py" "$W13_ARTIFACT_DIR"

trap - EXIT INT TERM

printf 'W13_ARTIFACT_DIR=%s\n' "$W13_ARTIFACT_DIR"
printf 'W13_CLEANUP_OK\n'
printf 'W13_CROSS_STACK_ACCEPTANCE_PASS\n'
