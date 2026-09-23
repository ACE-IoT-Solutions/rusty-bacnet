#!/usr/bin/env bash

set -euo pipefail

readonly DISTRIBUTION="ace-rusty-bacnet"
readonly IMPORT_MODULE="rusty_bacnet"
readonly REGISTRY_URL="https://ace-pypi.tail8c70f.ts.net/"
readonly INDEX_URL="${REGISTRY_URL}simple/"

usage() {
    echo "Usage: $0 <version> [--build-only] [--allow-dirty] [--wheel-dir DIR]" >&2
    echo "" >&2
    echo "Build and publish the ${DISTRIBUTION} Python release." >&2
    echo "--build-only validates artifacts without uploading them." >&2
    echo "--allow-dirty is accepted only with --build-only." >&2
    echo "--wheel-dir adds prebuilt wheels, such as manylinux artifacts." >&2
}

if [[ $# -lt 1 ]]; then
    usage
    exit 2
fi

readonly EXPECTED_VERSION="$1"
shift

publish=true
allow_dirty=false
wheel_dir=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --build-only) publish=false ;;
        --allow-dirty) allow_dirty=true ;;
        --wheel-dir)
            if [[ $# -lt 2 ]]; then
                usage
                exit 2
            fi
            wheel_dir="$2"
            shift
            ;;
        *)
            usage
            exit 2
            ;;
    esac
    shift
done

if [[ "$publish" == true && "$allow_dirty" == true ]]; then
    echo "error: --allow-dirty may only be used with --build-only" >&2
    exit 2
fi

for command_name in cargo cp git tar unzip uv; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "error: required command not found: ${command_name}" >&2
        exit 1
    fi
done

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"
python_executable=$(uv python find)

if [[ "$allow_dirty" == false ]] && [[ -n "$(git status --porcelain)" ]]; then
    echo "error: refusing to release from a dirty worktree" >&2
    exit 1
fi

release_commit=$(git rev-parse HEAD)
metadata=$(cargo metadata --locked --no-deps --format-version 1)
package_record=$("$python_executable" -c '
import json, sys
metadata = json.load(sys.stdin)
matches = [p for p in metadata["packages"] if p["name"] == "rusty-bacnet"]
if len(matches) != 1:
    raise SystemExit("expected exactly one rusty-bacnet Cargo package")
print(matches[0]["version"], matches[0]["manifest_path"], sep="\t")
' <<<"$metadata")
IFS=$'\t' read -r actual_version manifest_path <<<"$package_record"

if [[ "$actual_version" != "$EXPECTED_VERSION" ]]; then
    echo "error: requested version ${EXPECTED_VERSION}, Cargo metadata reports ${actual_version}" >&2
    exit 1
fi

crate_dir=$(dirname "$manifest_path")
project_name=$("$python_executable" -c '
import sys, tomllib
with open(sys.argv[1], "rb") as handle:
    print(tomllib.load(handle)["project"]["name"])
' "$crate_dir/pyproject.toml")
if [[ "$project_name" != "$DISTRIBUTION" ]]; then
    echo "error: expected distribution ${DISTRIBUTION}, found ${project_name}" >&2
    exit 1
fi

artifact_dir="$repo_root/dist/python/$EXPECTED_VERSION"
release_tmp=$(mktemp -d "${TMPDIR:-/tmp}/ace-rusty-bacnet-release.XXXXXX")
trap 'rm -rf "$release_tmp"' EXIT

# Stage supplemental wheels before uv clears the artifact directory. This also
# permits callers to keep temporary wheel inputs beneath dist/python/$VERSION.
staged_wheel_dir="$release_tmp/additional-wheels"
if [[ -n "$wheel_dir" ]]; then
    if [[ ! -d "$wheel_dir" ]]; then
        echo "error: wheel directory does not exist: ${wheel_dir}" >&2
        exit 1
    fi
    shopt -s nullglob
    additional_wheels=("$wheel_dir"/ace_rusty_bacnet-"$EXPECTED_VERSION"-*.whl)
    if [[ ${#additional_wheels[@]} -eq 0 ]]; then
        echo "error: no ${DISTRIBUTION} ${EXPECTED_VERSION} wheels found in ${wheel_dir}" >&2
        exit 1
    fi
    mkdir -p "$staged_wheel_dir"
    cp "${additional_wheels[@]}" "$staged_wheel_dir/"
fi

# Maturin rewrites workspace members in an sdist but currently copies the
# full-workspace Cargo.lock unchanged. Re-resolve only that staged workspace,
# prove it builds locked and offline, then create the final source archive.
stock_dir="$release_tmp/stock"
prepared_dir="$release_tmp/prepared"
uv build "$crate_dir" --python "$python_executable" --out-dir "$stock_dir" --sdist --clear
stock_sdists=("$stock_dir"/ace_rusty_bacnet-"$EXPECTED_VERSION".tar.gz)
if [[ ${#stock_sdists[@]} -ne 1 || ! -f "${stock_sdists[0]}" ]]; then
    echo "error: expected one staged source distribution" >&2
    exit 1
fi
mkdir -p "$prepared_dir"
tar -xzf "${stock_sdists[0]}" -C "$prepared_dir"
prepared_source="$prepared_dir/ace_rusty_bacnet-$EXPECTED_VERSION"
if [[ ! -d "$prepared_source" ]]; then
    echo "error: source distribution has an unexpected root directory" >&2
    exit 1
fi
(
    cd "$prepared_source"
    cargo check -p rusty-bacnet --lib
    cargo check --locked --offline -p rusty-bacnet --lib
)

uv build "$prepared_source" --python "$python_executable" --out-dir "$artifact_dir" --sdist --clear
sdists=("$artifact_dir"/ace_rusty_bacnet-"$EXPECTED_VERSION".tar.gz)
if [[ ${#sdists[@]} -ne 1 || ! -f "${sdists[0]}" ]]; then
    echo "error: expected one repaired source distribution" >&2
    exit 1
fi
uv build "${sdists[0]}" --python "$python_executable" --out-dir "$artifact_dir" --wheel

shopt -s nullglob
local_wheels=("$artifact_dir"/ace_rusty_bacnet-"$EXPECTED_VERSION"-*.whl)
if [[ ${#local_wheels[@]} -ne 1 ]]; then
    echo "error: expected one locally built wheel" >&2
    exit 1
fi

if [[ -d "$staged_wheel_dir" ]]; then
    staged_wheels=("$staged_wheel_dir"/*.whl)
    for staged_wheel in "${staged_wheels[@]}"; do
        destination="$artifact_dir/$(basename "$staged_wheel")"
        if [[ -e "$destination" ]]; then
            echo "error: supplemental wheel collides with a locally built wheel: ${destination}" >&2
            exit 1
        fi
        cp "$staged_wheel" "$destination"
    done
fi

wheels=("$artifact_dir"/ace_rusty_bacnet-"$EXPECTED_VERSION"-*.whl)
for wheel in "${wheels[@]}"; do
    metadata_name=$(unzip -p "$wheel" '*/METADATA' | awk -F': ' '$1 == "Name" { print $2; exit }' | tr -d '\r')
    metadata_version=$(unzip -p "$wheel" '*/METADATA' | awk -F': ' '$1 == "Version" { print $2; exit }' | tr -d '\r')
    if [[ "$metadata_name" != "$DISTRIBUTION" || "$metadata_version" != "$EXPECTED_VERSION" ]]; then
        echo "error: wheel metadata mismatch: ${metadata_name} ${metadata_version}" >&2
        exit 1
    fi
    if ! unzip -Z1 "$wheel" | grep -Eq "(^|/)${IMPORT_MODULE}([./]|$)"; then
        echo "error: wheel does not contain the ${IMPORT_MODULE} import module: ${wheel}" >&2
        exit 1
    fi
done

if [[ ${#wheels[@]} -eq 0 ]]; then
    echo "error: no wheels available for ${DISTRIBUTION} ${EXPECTED_VERSION}" >&2
    exit 1
fi

smoke_dir="$release_tmp/smoke"
uv venv --python "$python_executable" "$smoke_dir/venv"
uv pip install --python "$smoke_dir/venv/bin/python" --no-deps "${local_wheels[0]}"
(
    cd "$smoke_dir"
    PYTHONPATH='' "$smoke_dir/venv/bin/python" -I -c \
        "import ${IMPORT_MODULE}; print(${IMPORT_MODULE}.__name__)"
)

if [[ "$publish" == false ]]; then
    echo "validated ${DISTRIBUTION} ${EXPECTED_VERSION} in ${artifact_dir}"
    exit 0
fi

if [[ "$(git rev-parse HEAD)" != "$release_commit" ]] || [[ -n "$(git status --porcelain)" ]]; then
    echo "error: repository changed during the release build; refusing to publish" >&2
    exit 1
fi

# uv requires an authentication header even when pypiserver delegates access
# control to Tailscale. These non-secret placeholders are ignored by that
# configuration; explicitly configured registry credentials still take priority.
if [[ -z "${UV_PUBLISH_TOKEN:-}" && -z "${UV_PUBLISH_USERNAME:-}" && -z "${UV_PUBLISH_PASSWORD:-}" ]]; then
    export UV_PUBLISH_USERNAME="tailscale"
    export UV_PUBLISH_PASSWORD="tailscale"
fi

uv publish \
    --publish-url "$REGISTRY_URL" \
    --check-url "$INDEX_URL" \
    --trusted-publishing never \
    --no-attestations \
    "${sdists[0]}" "${wheels[@]}"

echo "published ${DISTRIBUTION} ${EXPECTED_VERSION} to ${REGISTRY_URL}"
