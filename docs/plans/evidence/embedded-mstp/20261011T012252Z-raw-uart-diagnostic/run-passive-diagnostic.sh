#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "usage: $0 CANDIDATE ROLLBACK SERIAL_DEVICE RUN_DIRECTORY" >&2
    echo "required environment: CANDIDATE_SHA256 ROLLBACK_SHA256" >&2
    exit 2
}

[[ $# -eq 4 ]] || usage
: "${CANDIDATE_SHA256:?CANDIDATE_SHA256 is required}"
: "${ROLLBACK_SHA256:?ROLLBACK_SHA256 is required}"

candidate="$1"
rollback="$2"
serial_device="$3"
run_dir="$4"
python_bin="${FLASH_PYTHON:-/home/ace/ace-router-hil/.flash-venv/bin/python3}"
capture_seconds="${CAPTURE_SECONDS:-40}"
node_f_url="${NODE_F_URL:-http://192.168.9.18}"
peer_url="${PEER_URL:-http://192.168.9.115}"
dashboard_service="${DASHBOARD_SERVICE:-ace-hil-dashboard.service}"
candidate_written=0
dashboard_was_active=0

sha256() {
    sha256sum "$1" | awk '{print $1}'
}

flash_image() {
    local image="$1"
    "${python_bin}" -m esptool --chip esp32s3 --port "${serial_device}" \
        --baud 460800 write-flash 0x0 "${image}"
}

snapshot() {
    local base_url="$1"
    local output="$2"
    curl --connect-timeout 3 --max-time 8 -fsS "${base_url}/statistics.json" >"${output}"
}

restore() {
    local status=$?
    trap - EXIT INT TERM
    if [[ ${candidate_written} -eq 1 ]]; then
        {
            echo "restoring rollback image"
            flash_image "${rollback}"
            sleep 8
            curl --connect-timeout 3 --max-time 8 -fsS "${node_f_url}/healthz"
            snapshot "${node_f_url}" "${run_dir}/node-f-restored.json"
        } >"${run_dir}/rollback.log" 2>&1 || status=1
    fi
    if [[ ${dashboard_was_active} -eq 1 ]]; then
        systemctl --user start "${dashboard_service}" || status=1
    fi
    exit "${status}"
}
trap restore EXIT INT TERM

[[ -f "${candidate}" ]] || { echo "candidate not found: ${candidate}" >&2; exit 1; }
[[ -f "${rollback}" ]] || { echo "rollback not found: ${rollback}" >&2; exit 1; }
[[ -e "${serial_device}" ]] || { echo "serial device not found: ${serial_device}" >&2; exit 1; }
[[ "$(sha256 "${candidate}")" == "${CANDIDATE_SHA256}" ]] || {
    echo "candidate checksum mismatch" >&2
    exit 1
}
[[ "$(sha256 "${rollback}")" == "${ROLLBACK_SHA256}" ]] || {
    echo "rollback checksum mismatch" >&2
    exit 1
}

mkdir -p "${run_dir}"
snapshot "${node_f_url}" "${run_dir}/node-f-before.json"
snapshot "${peer_url}" "${run_dir}/peer-e-before.json"

{
    echo "candidate_sha256=${CANDIDATE_SHA256}"
    echo "rollback_sha256=${ROLLBACK_SHA256}"
    echo "serial_device=${serial_device}"
    echo "capture_seconds=${capture_seconds}"
    date -u '+started_at=%FT%TZ'
} >"${run_dir}/preflight.txt"

if systemctl --user is-active --quiet "${dashboard_service}"; then
    dashboard_was_active=1
    systemctl --user stop "${dashboard_service}"
fi
if fuser "${serial_device}" >"${run_dir}/serial-owner.txt" 2>&1; then
    echo "serial device remains busy after stopping the dashboard" >&2
    exit 1
fi

flash_image "${candidate}" >"${run_dir}/candidate-flash.log" 2>&1
candidate_written=1
sleep 2

"${python_bin}" - "${serial_device}" "${capture_seconds}" \
    >"${run_dir}/serial-F.log" 2>"${run_dir}/serial-capture.err" <<'PY'
import sys
import time

import serial

device = sys.argv[1]
duration = float(sys.argv[2])
deadline = time.monotonic() + duration
with serial.Serial(device, 115200, timeout=0.2, exclusive=True) as stream:
    while time.monotonic() < deadline:
        line = stream.readline()
        if line:
            text = line.decode("utf-8", errors="replace").rstrip("\r\n")
            print(f"{time.time_ns()} {text}", flush=True)
PY

snapshot "${peer_url}" "${run_dir}/peer-e-after.json"
date -u '+finished_at=%FT%TZ' >>"${run_dir}/preflight.txt"
