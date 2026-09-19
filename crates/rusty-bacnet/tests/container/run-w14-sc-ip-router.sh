#!/bin/sh
set -eu

command -v podman >/dev/null 2>&1 || { echo "podman is required" >&2; exit 127; }
podman compose version >/dev/null 2>&1 || { echo "podman compose is required" >&2; exit 127; }
command -v git >/dev/null 2>&1 || { echo "git is required" >&2; exit 127; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required" >&2; exit 127; }

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/../../../.." && pwd)
compose_file="$script_dir/compose.w14-sc-ip-router.yml"
project_name="rusty-bacnet-w14-sc-ip-router-$$"
wheel_image="localhost/rusty-bacnet-w14-wheel:dev"
fixture_image="localhost/rusty-bacnet-w14-sc-ip-router:dev"
metadata_container="${project_name}_metadata"
W14_ARTIFACT_DIR=${W14_ARTIFACT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/rusty-bacnet-w14-sc-ip-router.XXXXXX")}
export W14_ARTIFACT_DIR

compose() {
    podman compose --project-name "$project_name" --file "$compose_file" "$@"
}

container_id() {
    printf '%s_%s_1\n' "$project_name" "$1"
}

capture_logs() {
    compose logs --no-color sc_ip_router >"$W14_ARTIFACT_DIR/sc-ip-router.log" 2>&1 || true
}

cleanup() {
    capture_logs
    podman rm --force "$metadata_container" >/dev/null 2>&1 || true
    if podman container exists "$(container_id sc_ip_router)"; then
        podman inspect "$(container_id sc_ip_router)" \
            >"$W14_ARTIFACT_DIR/container-inspect.json" 2>/dev/null || true
    fi
    compose down --volumes --remove-orphans >/dev/null 2>&1 || true
}

wait_for_container_exit() {
    container=$1 attempts=$2 count=0
    while [ "$count" -lt "$attempts" ]; do
        if ! podman container exists "$container"; then
            echo "container disappeared before its exit status was recorded: $container" >&2
            return 1
        fi
        running=$(podman inspect --format '{{.State.Running}}' "$container")
        if [ "$running" != "true" ]; then
            podman inspect --format '{{.State.ExitCode}}' "$container"
            return 0
        fi
        sleep 1
        count=$((count + 1))
    done
    echo "timed out waiting for container to exit: $container" >&2
    return 1
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

run_build() {
    name=$1
    shift
    log="$W14_ARTIFACT_DIR/$name-build.log"
    if "$@" >"$log" 2>&1; then
        printf '%s_exit_code=0\n' "$name" >>"$W14_ARTIFACT_DIR/build-exit-codes.txt"
        tail -20 "$log"
    else
        status=$?
        printf '%s_exit_code=%s\n' "$name" "$status" >>"$W14_ARTIFACT_DIR/build-exit-codes.txt"
        tail -80 "$log" >&2 || true
        return "$status"
    fi
}

mkdir -p "$W14_ARTIFACT_DIR"
: >"$W14_ARTIFACT_DIR/runner-transcript.log"
: >"$W14_ARTIFACT_DIR/build-exit-codes.txt"
source_sha=$(git -C "$repo_root" rev-parse HEAD)
source_ref=$(git -C "$repo_root" symbolic-ref --quiet --short HEAD || git -C "$repo_root" rev-parse --short HEAD)
if git -C "$repo_root" diff --quiet HEAD -- && \
    test -z "$(git -C "$repo_root" ls-files --others --exclude-standard)"; then
    source_dirty=false
else
    source_dirty=true
fi
git -C "$repo_root" status --short >"$W14_ARTIFACT_DIR/source-status.txt"
git -C "$repo_root" diff --binary HEAD >"$W14_ARTIFACT_DIR/source-working-tree.patch"
git -C "$repo_root" ls-files --others --exclude-standard >"$W14_ARTIFACT_DIR/source-untracked-files.txt"
source_archive_sha256=$(python3 - "$repo_root" "$W14_ARTIFACT_DIR/source-files.json" <<'PY'
import hashlib
import json
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1])
output = Path(sys.argv[2])
paths = subprocess.check_output(
    ["git", "-C", str(root), "ls-files", "-z", "--cached", "--others", "--exclude-standard"]
).decode().rstrip("\0").split("\0")

def in_build_context(name: str) -> bool:
    path = Path(name)
    if not name or path.parts[0] in {
        "target",
        "_spec",
        "docs",
        ".github",
        ".cocoindex_code",
    }:
        return False
    if path.name == ".dockerignore" or path.suffix.lower() == ".md":
        return False
    return path.suffix.lower() not in {".pem", ".key", ".csr", ".der", ".p12", ".pfx", ".srl"}

files = []
for name in sorted(filter(in_build_context, paths)):
    path = root / name
    if path.is_file():
        data = path.read_bytes()
        files.append({"path": name, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
payload = {"schema_version": 1, "files": files}
canonical = json.dumps(payload, separators=(",", ":"), sort_keys=True).encode()
payload["snapshot_sha256"] = hashlib.sha256(canonical).hexdigest()
output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
print(payload["snapshot_sha256"])
PY
)
printf '%s\n' \
    "source_sha=$source_sha" \
    "source_ref=$source_ref" \
    "source_dirty=$source_dirty" \
    "source_build_context_sha256=$source_archive_sha256" \
    >"$W14_ARTIFACT_DIR/source-metadata.txt"

if [ "${W14_SKIP_BUILD:-0}" != "1" ]; then
    run_build wheel podman build --tag "$wheel_image" \
        --file "$script_dir/Dockerfile.sc-runtime" \
        --build-arg SOURCE_SHA="$source_sha" \
        --build-arg SOURCE_REF="$source_ref" \
        --build-arg SOURCE_DIRTY="$source_dirty" \
        --build-arg SOURCE_ARCHIVE_SHA256="$source_archive_sha256" \
        "$repo_root"
    run_build fixture podman build --tag "$fixture_image" \
        --file "$script_dir/Dockerfile.w14-sc-ip-router" \
        --build-arg WHEEL_IMAGE="$wheel_image" \
        "$repo_root"
fi

podman image inspect "$wheel_image" >"$W14_ARTIFACT_DIR/wheel-image-inspect.json"
podman image inspect "$fixture_image" >"$W14_ARTIFACT_DIR/fixture-image-inspect.json"
podman create --name "$metadata_container" "$fixture_image" >/dev/null
podman cp \
    "$metadata_container:/usr/local/share/rusty-bacnet/build-environment.txt" \
    "$W14_ARTIFACT_DIR/build-environment.txt"
podman rm "$metadata_container" >/dev/null
retained_source_archive_sha256=$(sed -n 's/^source_archive_sha256=//p' \
    "$W14_ARTIFACT_DIR/build-environment.txt")
if [ -z "$retained_source_archive_sha256" ] || \
    [ "$retained_source_archive_sha256" != "$source_archive_sha256" ]; then
    echo "retained wheel source archive does not match the current build context" >&2
    echo "current_source_build_context_sha256=$source_archive_sha256" >&2
    echo "retained_source_archive_sha256=${retained_source_archive_sha256:-missing}" >&2
    exit 1
fi
compose config >"$W14_ARTIFACT_DIR/compose-config.yml"
compose up --detach --no-deps sc_ip_router
fixture=$(container_id sc_ip_router)
if ! marker=$(wait_for_log "$fixture" W14_POST_HUB_RESTART_ROUTED_READ_PASS 120); then
    exit 1
fi
printf '%s\n' "$marker" | tee -a "$W14_ARTIFACT_DIR/runner-transcript.log"
if ! marker=$(wait_for_log "$fixture" W14_SC_IP_ROUTER_ACCEPTANCE_PASS 2); then
    exit 1
fi
printf '%s\n' "$marker" | tee -a "$W14_ARTIFACT_DIR/runner-transcript.log"
if ! exit_code=$(wait_for_container_exit "$fixture" 30); then
    exit 1
fi
podman inspect "$fixture" >"$W14_ARTIFACT_DIR/container-inspect.json"
capture_logs
python3 - "$W14_ARTIFACT_DIR" "$project_name" "$exit_code" <<'PY'
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
(root / "runner-result.json").write_text(
    json.dumps(
        {
            "schema_version": 1,
            "project": sys.argv[2],
            "container_exit_code": int(sys.argv[3]),
            "post_hub_restart_marker": "W14_POST_HUB_RESTART_ROUTED_READ_PASS",
            "acceptance_marker": "W14_SC_IP_ROUTER_ACCEPTANCE_PASS",
            "service_log": "sc-ip-router.log",
        },
        indent=2,
        sort_keys=True,
    )
    + "\n"
)
PY
if [ "$exit_code" -ne 0 ]; then
    podman logs "$fixture" >&2 || true
    exit "$exit_code"
fi
python3 -m json.tool "$W14_ARTIFACT_DIR/w14-sc-ip-router-results.json" >/dev/null
test -s "$W14_ARTIFACT_DIR/wheel.sha256"
test -s "$W14_ARTIFACT_DIR/build-environment.txt"

compose down --volumes --remove-orphans
trap - EXIT INT TERM
if podman container exists "$fixture"; then
    echo "W14 container remained after cleanup: $fixture" >&2
    exit 1
fi
if podman network exists "${project_name}_sc_ip_router"; then
    echo "W14 network remained after cleanup: ${project_name}_sc_ip_router" >&2
    exit 1
fi
printf '%s\n' W14_CLEANUP_OK | tee -a "$W14_ARTIFACT_DIR/runner-transcript.log"
printf '%s\n' W14_SC_IP_ROUTER_ACCEPTANCE_PASS | tee -a "$W14_ARTIFACT_DIR/runner-transcript.log"
printf 'W14_SC_IP_ROUTER_ARTIFACT_DIR=%s\n' "$W14_ARTIFACT_DIR"
