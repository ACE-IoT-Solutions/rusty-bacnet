#!/usr/bin/env bash

set -euo pipefail

readonly DISTRIBUTION="ace-rusty-bacnet"
readonly IMPORT_MODULE="rusty_bacnet"

usage() {
    echo "Usage: $0 <version> [--allow-dirty] [--wheel-dir DIR]" >&2
    echo "" >&2
    echo "Build and validate local ${DISTRIBUTION} Python artifacts." >&2
    echo "--allow-dirty permits local changes for a validation build." >&2
    echo "--wheel-dir adds prebuilt wheels, such as manylinux artifacts." >&2
}

if [[ $# -lt 1 ]]; then
    usage
    exit 2
fi

readonly EXPECTED_VERSION="$1"
shift

allow_dirty=false
wheel_dir=""
while [[ $# -gt 0 ]]; do
    case "$1" in
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

for command_name in cargo cp git tar unzip uv; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "error: required command not found: ${command_name}" >&2
        exit 1
    fi
done

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"
python_executable=$(uv python find)
# The release helper reads TOML with the standard library. Its wheel may target 3.10.
metadata_python=$(uv python find '>=3.11')

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
project_name=$("$metadata_python" -c '
import sys, tomllib
with open(sys.argv[1], "rb") as handle:
    print(tomllib.load(handle)["project"]["name"])
' "$crate_dir/pyproject.toml")
if [[ "$project_name" != "$DISTRIBUTION" ]]; then
    echo "error: expected distribution ${DISTRIBUTION}, found ${project_name}" >&2
    exit 1
fi

python_version=$("$metadata_python" -c '
import sys
sys.path.insert(0, "scripts/release")
from check_artifacts import pep440
print(pep440(sys.argv[1]))
' "$EXPECTED_VERSION")
artifact_parent="$repo_root/dist/python-build/$EXPECTED_VERSION"
release_tmp=$(mktemp -d "${TMPDIR:-/tmp}/ace-rusty-bacnet-release.XXXXXX")
trap 'rm -rf "$release_tmp"' EXIT

# Snapshot supplemental wheels before validating the complete local build set.
staged_wheel_dir="$release_tmp/additional-wheels"
if [[ -n "$wheel_dir" ]]; then
    if [[ ! -d "$wheel_dir" ]]; then
        echo "error: wheel directory does not exist: ${wheel_dir}" >&2
        exit 1
    fi
    shopt -s nullglob
    additional_wheels=("$wheel_dir"/ace_rusty_bacnet-"$python_version"-*.whl)
    if [[ ${#additional_wheels[@]} -eq 0 ]]; then
        echo "error: no ${DISTRIBUTION} ${EXPECTED_VERSION} wheels found in ${wheel_dir}" >&2
        exit 1
    fi
    mkdir -p "$staged_wheel_dir"
    cp "${additional_wheels[@]}" "$staged_wheel_dir/"
fi

# Use the same pinned builder and offline repair as the ACE release source job.
tools_dir="$release_tmp/tools"
uv venv --python "$metadata_python" "$tools_dir"
uv pip install --python "$tools_dir/bin/python" --require-hashes --only-binary :all: \
    -r scripts/release/maturin-requirements.txt
export SOURCE_DATE_EPOCH
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct "$release_commit")
assembled="$release_tmp/artifacts"
mkdir -p "$assembled"
PATH="$tools_dir/bin:$PATH" "$metadata_python" scripts/release/build_ace_sdist.py "$EXPECTED_VERSION" "$assembled"
sdist="$assembled/ace_rusty_bacnet-$python_version.tar.gz"
mkdir "$release_tmp/source"
tar -xzf "$sdist" -C "$release_tmp/source"
(
    cd "$release_tmp/source/ace_rusty_bacnet-$python_version"
    cargo check --locked --offline -p rusty-bacnet --lib
    "$tools_dir/bin/maturin" build --locked --offline --release --interpreter "$python_executable" --out "$assembled"
)

shopt -s nullglob
local_wheels=("$assembled"/ace_rusty_bacnet-"$python_version"-*.whl)
if [[ ${#local_wheels[@]} -ne 1 ]]; then
    echo "error: expected one locally built wheel" >&2
    exit 1
fi

if [[ -d "$staged_wheel_dir" ]]; then
    staged_wheels=("$staged_wheel_dir"/*.whl)
    for staged_wheel in "${staged_wheels[@]}"; do
        destination="$assembled/$(basename "$staged_wheel")"
        if [[ -e "$destination" ]]; then
            echo "error: supplemental wheel collides with a locally built wheel: ${destination}" >&2
            exit 1
        fi
        cp "$staged_wheel" "$destination"
    done
fi

wheels=("$assembled"/ace_rusty_bacnet-"$python_version"-*.whl)
for wheel in "${wheels[@]}"; do
    metadata_name=$(unzip -p "$wheel" '*/METADATA' | awk -F': ' '$1 == "Name" { print $2; exit }' | tr -d '\r')
    metadata_version=$(unzip -p "$wheel" '*/METADATA' | awk -F': ' '$1 == "Version" { print $2; exit }' | tr -d '\r')
    if [[ "$metadata_name" != "$DISTRIBUTION" || "$metadata_version" != "$python_version" ]]; then
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

if [[ "$(git rev-parse HEAD)" != "$release_commit" ]] || \
   { [[ "$allow_dirty" == false ]] && [[ -n "$(git status --porcelain)" ]]; }; then
    echo "error: repository changed during the validation build" >&2
    exit 1
fi
mkdir -p "$artifact_parent"
artifact_dir=$(mktemp -d "$artifact_parent/validated.XXXXXX")
cp "$sdist" "${wheels[@]}" "$artifact_dir/"
echo "validated ${DISTRIBUTION} ${EXPECTED_VERSION} in ${artifact_dir}"
