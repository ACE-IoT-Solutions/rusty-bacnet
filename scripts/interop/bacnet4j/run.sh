#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 3 || $# -gt 4 ]]; then
    echo "usage: $0 WHEEL SOURCE_SHA OUTPUT_JSON [--reads-only]" >&2
    exit 2
fi
wheel=$(realpath "$1")
source_sha=$2
output_dir=$(cd "$(dirname "$3")" && pwd)
output_name=$(basename "$3")
fixture=$(cd "$(dirname "$0")" && pwd)
peer_image=$(podman image inspect --format '{{.Id}}' "${BACNET4J_IMAGE:-localhost/rusty-bacnet-interop-bacnet4j:6.1.1}")
client_image=$(podman image inspect --format '{{.Id}}' "${RUSTY_QA_IMAGE:-localhost/rusty-bacnet-downstream-qa:rust1.99-py313}")
peer=$(podman run -d --network none "$peer_image")
trap 'podman rm -f "$peer" >/dev/null' EXIT
for ((attempt=0; attempt<60; attempt++)); do
    if podman logs "$peer" 2>&1 | grep -q 'BACnet4J peer ready'; then break; fi
    if [[ $(podman inspect --format '{{.State.Running}}' "$peer") != true ]]; then
        podman logs "$peer" >&2
        exit 1
    fi
    sleep 1
done

podman run --rm --network "container:$peer" --entrypoint /bin/bash \
    -v "$fixture:/probe:ro" -v "$wheel:/wheel/$(basename "$wheel"):ro" -v "$output_dir:/output:rw" \
    "$client_image" -ec '
        python3.13 -m venv /tmp/probe
        /tmp/probe/bin/python -m pip --disable-pip-version-check install --no-index --no-deps "$1"
        exec /tmp/probe/bin/python /probe/probe.py --wheel "$1" --source-sha "$2" --output "$3" \
            --peer-image "$4" --client-image "$5" "${@:6}"
    ' probe "/wheel/$(basename "$wheel")" "$source_sha" "/output/$output_name" "$peer_image" "$client_image" "${@:4}" \
    || { podman logs "$peer" >&2; exit 1; }
