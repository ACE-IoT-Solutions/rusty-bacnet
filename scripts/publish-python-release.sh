#!/usr/bin/env bash

set -euo pipefail

readonly DISTRIBUTION="ace-rusty-bacnet"
readonly IMPORT_MODULE="rusty_bacnet"
readonly REGISTRY_URL="https://ace-pypi.tail8c70f.ts.net/"
readonly INDEX_URL="${REGISTRY_URL}simple/"

usage() {
    echo "Usage: $0 <version> [--build-only] [--allow-dirty]" >&2
    echo "" >&2
    echo "Build and publish the ${DISTRIBUTION} Python release." >&2
    echo "--build-only validates artifacts without uploading them." >&2
    echo "--allow-dirty is accepted only with --build-only." >&2
}

if [[ $# -lt 1 || $# -gt 3 ]]; then
    usage
    exit 2
fi

readonly EXPECTED_VERSION="$1"
shift

publish=true
allow_dirty=false
for option in "$@"; do
    case "$option" in
        --build-only) publish=false ;;
        --allow-dirty) allow_dirty=true ;;
        *)
            usage
            exit 2
            ;;
    esac
done

if [[ "$publish" == true && "$allow_dirty" == true ]]; then
    echo "error: --allow-dirty may only be used with --build-only" >&2
    exit 2
fi

for command_name in cargo git unzip uv; do
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
uv build "$crate_dir" --python "$python_executable" --out-dir "$artifact_dir" --sdist --clear
uv build "$crate_dir" --python "$python_executable" --out-dir "$artifact_dir" --wheel

shopt -s nullglob
wheels=("$artifact_dir"/ace_rusty_bacnet-"$EXPECTED_VERSION"-*.whl)
sdists=("$artifact_dir"/ace_rusty_bacnet-"$EXPECTED_VERSION".tar.gz)
if [[ ${#wheels[@]} -ne 1 || ${#sdists[@]} -ne 1 ]]; then
    echo "error: expected one wheel and one source distribution for ${DISTRIBUTION} ${EXPECTED_VERSION}" >&2
    exit 1
fi

metadata_name=$(unzip -p "${wheels[0]}" '*/METADATA' | awk -F': ' '$1 == "Name" { print $2; exit }' | tr -d '\r')
metadata_version=$(unzip -p "${wheels[0]}" '*/METADATA' | awk -F': ' '$1 == "Version" { print $2; exit }' | tr -d '\r')
if [[ "$metadata_name" != "$DISTRIBUTION" || "$metadata_version" != "$EXPECTED_VERSION" ]]; then
    echo "error: wheel metadata mismatch: ${metadata_name} ${metadata_version}" >&2
    exit 1
fi
if ! unzip -Z1 "${wheels[0]}" | grep -Eq "(^|/)${IMPORT_MODULE}([./]|$)"; then
    echo "error: wheel does not contain the ${IMPORT_MODULE} import module" >&2
    exit 1
fi

smoke_dir=$(mktemp -d "${TMPDIR:-/tmp}/ace-rusty-bacnet-release.XXXXXX")
trap 'rm -rf "$smoke_dir"' EXIT
uv venv --python "$python_executable" "$smoke_dir/venv"
uv pip install --python "$smoke_dir/venv/bin/python" --no-deps "${wheels[0]}"
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
    "${sdists[0]}" "${wheels[0]}"

echo "published ${DISTRIBUTION} ${EXPECTED_VERSION} to ${REGISTRY_URL}"
