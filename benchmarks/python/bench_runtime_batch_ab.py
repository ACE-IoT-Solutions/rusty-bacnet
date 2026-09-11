"""Correctness-gated clean-wheel runtime versus chatty-client B/IP A/B harness."""

from __future__ import annotations

import asyncio
import hashlib
import json
import math
import os
import platform
import resource
import statistics
import sys
import time
from pathlib import Path
from typing import Any, Awaitable, Callable

import rusty_bacnet as bacnet


INTERFACE = os.environ["BACNET_INTERFACE"]
BROADCAST = os.environ["BACNET_BROADCAST"]
PORT = int(os.environ.get("BACNET_PORT", "47808"))
DEVICE_INSTANCE = int(os.environ.get("BACNET_DEVICE_INSTANCE", "4106"))
SERVER_ADDRESS = os.environ.get("BACNET_SERVER_ADDRESS", "172.32.0.10:47808")
WARMUPS = int(os.environ.get("BACNET_BENCH_WARMUPS", "20"))
SAMPLES = int(os.environ.get("BACNET_BENCH_SAMPLES", "200"))
TRIALS = int(os.environ.get("BACNET_BENCH_TRIALS", "5"))
CANCEL_OPERATIONS = int(os.environ.get("BACNET_BENCH_CANCEL_OPERATIONS", "64"))
RSS_INTERVAL = int(os.environ.get("BACNET_BENCH_RSS_INTERVAL_MS", "10")) / 1_000
OUTPUT = Path(os.environ.get("BACNET_BENCH_OUTPUT", "/results/runtime-batch-ab.json"))
WHEEL_DIGEST_FILE = Path("/usr/local/share/rusty-bacnet/wheel.sha256")
BUILD_ENVIRONMENT_FILE = Path("/usr/local/share/rusty-bacnet/build-environment.txt")
HARNESS_FILE = Path(__file__).resolve()

OBJECTS = [bacnet.ObjectIdentifier(bacnet.ObjectType.ANALOG_INPUT, index) for index in range(3)]
PROPERTIES = [bacnet.PropertyIdentifier.PRESENT_VALUE, bacnet.PropertyIdentifier.OBJECT_NAME]
EXPECTED = [72.5, "AI-0", 68.0, "AI-1", 55.0, "AI-2"]


def require_positive_configuration() -> None:
    for name, value in {
        "warmups": WARMUPS,
        "samples": SAMPLES,
        "trials": TRIALS,
        "cancel_operations": CANCEL_OPERATIONS,
        "rss_interval_ms": int(RSS_INTERVAL * 1_000),
    }.items():
        if value <= 0:
            raise AssertionError(f"{name} must be positive, got {value}")


def percentile(samples: list[int], fraction: float) -> int:
    ordered = sorted(samples)
    return ordered[min(len(ordered) - 1, math.ceil(len(ordered) * fraction) - 1)]


def summary(samples: list[int]) -> dict[str, float | int]:
    seconds = sum(samples) / 1_000_000_000
    return {
        "p50_ns": percentile(samples, 0.50),
        "p95_ns": percentile(samples, 0.95),
        "p99_ns": percentile(samples, 0.99),
        "max_ns": max(samples),
        "workflows_per_second": len(samples) / seconds,
        "properties_per_second": len(samples) * len(EXPECTED) / seconds,
    }


def assert_values(values: list[object]) -> None:
    if len(values) != len(EXPECTED):
        raise AssertionError(f"expected six values, got {values!r}")
    for actual, expected in zip(values, EXPECTED):
        if isinstance(expected, float):
            actual_number = float(actual)
            if not math.isfinite(actual_number) or abs(actual_number - expected) > 0.001:
                raise AssertionError(f"value mismatch: {actual!r} != {expected!r}")
        elif actual != expected:
            raise AssertionError(f"value mismatch: {actual!r} != {expected!r}")


def parse_key_values(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in path.read_text().splitlines():
        key, separator, value = line.partition("=")
        if not separator or not key:
            raise AssertionError(f"malformed identity line in {path}: {line!r}")
        result[key] = value
    return result


def fd_snapshot() -> dict[str, Any]:
    entries: list[dict[str, str]] = []
    try:
        paths = sorted(Path("/proc/self/fd").iterdir(), key=lambda path: int(path.name))
    except OSError:
        return {"available": False, "total": None, "classes": {}, "entries": []}
    for path in paths:
        try:
            target = os.readlink(path)
        except OSError:
            continue
        if target.startswith("socket:"):
            kind = "socket"
        elif target.startswith("pipe:"):
            kind = "pipe"
        elif target.startswith("anon_inode:"):
            kind = "anon_inode"
        elif target.startswith("/"):
            kind = "file"
        else:
            kind = "other"
        entries.append({"fd": path.name, "class": kind, "target": target})
    classes = {kind: sum(entry["class"] == kind for entry in entries) for kind in sorted({entry["class"] for entry in entries})}
    return {"available": True, "total": len(entries), "classes": classes, "entries": entries}


def linux_memory_snapshot() -> dict[str, int | None]:
    status: dict[str, int] = {}
    try:
        for line in Path("/proc/self/status").read_text().splitlines():
            if line.startswith(("VmRSS:", "VmHWM:")):
                key, value, _unit = line.split()
                status[key[:-1]] = int(value) * 1_024
    except OSError:
        pass
    return {"rss_bytes": status.get("VmRSS"), "peak_rss_bytes": status.get("VmHWM")}


def resource_snapshot() -> dict[str, Any]:
    usage = resource.getrusage(resource.RUSAGE_SELF)
    memory = linux_memory_snapshot()
    return {
        "monotonic_ns": time.monotonic_ns(),
        "cpu_seconds": usage.ru_utime + usage.ru_stime,
        "voluntary_context_switches": usage.ru_nvcsw,
        "involuntary_context_switches": usage.ru_nivcsw,
        **memory,
        "file_descriptors": fd_snapshot(),
    }


def resource_delta(before: dict[str, Any], after: dict[str, Any]) -> dict[str, Any]:
    return {
        "cpu_seconds": after["cpu_seconds"] - before["cpu_seconds"],
        "voluntary_context_switches": after["voluntary_context_switches"] - before["voluntary_context_switches"],
        "involuntary_context_switches": after["involuntary_context_switches"] - before["involuntary_context_switches"],
        "rss_bytes": None if before["rss_bytes"] is None or after["rss_bytes"] is None else after["rss_bytes"] - before["rss_bytes"],
        "file_descriptors": None if before["file_descriptors"]["total"] is None else after["file_descriptors"]["total"] - before["file_descriptors"]["total"],
        "sockets": after["file_descriptors"]["classes"].get("socket", 0) - before["file_descriptors"]["classes"].get("socket", 0),
    }


async def sample_rss(stop: asyncio.Event, samples: list[dict[str, int | None]]) -> None:
    while not stop.is_set():
        # Keep the sampler narrow so FD enumeration and getrusage do not add
        # avoidable noise to the latency arm being measured.
        samples.append(
            {
                "monotonic_ns": time.monotonic_ns(),
                "rss_bytes": linux_memory_snapshot()["rss_bytes"],
            }
        )
        try:
            await asyncio.wait_for(stop.wait(), timeout=RSS_INTERVAL)
        except TimeoutError:
            pass


async def measure(workflow: Callable[[], Awaitable[Any]]) -> dict[str, Any]:
    for _ in range(WARMUPS):
        await workflow()
    before = resource_snapshot()
    rss_samples: list[dict[str, int | None]] = []
    stop = asyncio.Event()
    sampler = asyncio.create_task(sample_rss(stop, rss_samples))
    measured: list[int] = []
    try:
        for _ in range(SAMPLES):
            started = time.perf_counter_ns()
            await workflow()
            measured.append(time.perf_counter_ns() - started)
    finally:
        stop.set()
        await sampler
    after = resource_snapshot()
    return {
        "samples_ns": measured,
        "rss_samples": rss_samples,
        "resources_before": before,
        "resources_after": after,
        "resource_delta": resource_delta(before, after),
        **summary(measured),
    }


def health_snapshot(health: Any) -> dict[str, Any]:
    return {
        "generation": health.generation,
        "accepting_commands": health.accepting_commands,
        "task_count": health.task_count,
        "supervisor_task_count": health.supervisor_task_count,
        "attachment_task_count": health.attachment_task_count,
        "event_queue_depth": health.event_queue_depth,
        "event_lag_count": health.event_lag_count,
        "cov_notification_lag_count": health.cov_notification_lag_count,
        "i_am_observation_lag_count": health.i_am_observation_lag_count,
        "device_count": health.device_count,
        "device_observation_count": health.device_observation_count,
        "capability_count": health.capability_count,
        "cached_value_count": health.cached_value_count,
        "observation_count": health.observation_count,
        "attachment_ids": health.attachment_ids,
        "attachment_states": health.attachment_states,
        "attachment_error_codes": health.attachment_error_codes,
    }


def assert_fd_cleanup(before: dict[str, Any], after: dict[str, Any], label: str) -> dict[str, Any]:
    before_fds = before["file_descriptors"]
    after_fds = after["file_descriptors"]
    if not before_fds["available"] or not after_fds["available"]:
        raise AssertionError(f"/proc FD classification unavailable for {label}")
    before_sockets = before_fds["classes"].get("socket", 0)
    after_sockets = after_fds["classes"].get("socket", 0)
    passed = before_fds["total"] == after_fds["total"] and before_sockets == after_sockets
    evidence = {
        "label": label,
        "passed": passed,
        "before": before_fds,
        "after": after_fds,
    }
    if not passed:
        raise AssertionError(f"descriptor/socket cleanup failed for {label}: {evidence!r}")
    return evidence


async def run_server() -> None:
    server = bacnet.BACnetServer(DEVICE_INSTANCE, "Runtime A/B fixture", interface=INTERFACE, port=PORT, broadcast_address=BROADCAST)
    for index, value in enumerate([72.5, 68.0, 55.0]):
        server.add_analog_input(index, f"AI-{index}", present_value=value)
    await server.start()
    try:
        await asyncio.Event().wait()
    finally:
        await server.stop()


async def chatty_workflow(client: bacnet.BACnetClient, gather: bool = False) -> list[object]:
    calls = [client.read_property(SERVER_ADDRESS, object_id, property_id) for object_id in OBJECTS for property_id in PROPERTIES]
    results = await asyncio.gather(*calls) if gather else [await call for call in calls]
    values = [result.value for result in results]
    assert_values(values)
    return values


def runtime_reads() -> list[Any]:
    reads = []
    index = 0
    for object_instance in range(3):
        for property_id, category in ((85, "real"), (77, "character_string")):
            reads.append(bacnet.RuntimeRead(index, 46, DEVICE_INSTANCE, 0, object_instance, property_id, category))
            index += 1
    return reads


def assert_runtime_outcomes(outcomes: list[Any]) -> list[object]:
    if [outcome.input_index for outcome in outcomes] != list(range(6)):
        raise AssertionError("runtime reordered outcomes")
    if any(outcome.error_code is not None for outcome in outcomes):
        raise AssertionError(f"runtime batch errors: {[outcome.error_code for outcome in outcomes]!r}")
    values = [outcome.value.value for outcome in outcomes]
    assert_values(values)
    return values


async def runtime_workflow(runtime: Any) -> list[object]:
    outcomes = await runtime.read_batch(runtime_reads(), timeout_ms=3_000)
    return assert_runtime_outcomes(outcomes)


async def start_discovered_runtime() -> Any:
    runtime = await bacnet.BACnetRuntime.start([bacnet.RuntimeAttachment(46, "benchmark", INTERFACE, PORT, BROADCAST)])
    for _ in range(5):
        snapshot = await runtime.discover(timeout_ms=300, low_limit=DEVICE_INSTANCE, high_limit=DEVICE_INSTANCE)
        if any(device.device_instance == DEVICE_INSTANCE for device in snapshot.devices):
            return runtime
    await runtime.stop()
    raise AssertionError("runtime did not discover benchmark server")


async def stabilize_native_runtime() -> dict[str, Any]:
    before = resource_snapshot()
    runtime = await bacnet.BACnetRuntime.start([bacnet.RuntimeAttachment(46, "stabilize", INTERFACE, 0, BROADCAST)])
    await runtime.stop()
    await asyncio.sleep(0.05)
    after = resource_snapshot()
    return {"before": before, "after": after, "purpose": "initialize process-global native runtime resources before cleanup baselines"}


async def submit_cancel_correctness_arm() -> dict[str, Any]:
    process_before = resource_snapshot()
    runtime = await start_discovered_runtime()
    health_before = health_snapshot(await runtime.health())
    try:
        operations = []
        reports = []
        # Cancel each operation immediately after submission. Submitting the
        # entire set before cancelling made this guard depend on server speed:
        # a fast local server could finish all batches before cancellation was
        # attempted, even though cancellation itself remained correct.
        for _ in range(CANCEL_OPERATIONS):
            operation = await runtime.submit_read_batch(
                runtime_reads(), timeout_ms=3_000, priority="background"
            )
            operations.append(operation)
            reports.append(await runtime.cancel(operation.operation_id))
        terminals = await asyncio.gather(*[operation.result() for operation in operations], return_exceptions=True)
        found = sum(report.found for report in reports)
        if found == 0:
            raise AssertionError("submit/cancel arm did not observe a queued or in-flight cancellation")
        terminal_kinds = {
            "outcomes": sum(isinstance(value, list) for value in terminals),
            "exceptions": sum(isinstance(value, BaseException) for value in terminals),
        }
        if sum(terminal_kinds.values()) != CANCEL_OPERATIONS:
            raise AssertionError("not every submitted operation reached a terminal result")
        for terminal in terminals:
            if isinstance(terminal, list):
                assert_runtime_outcomes(terminal)
            elif getattr(terminal, "code", None) != "Cancelled":
                raise AssertionError(f"unexpected cancellation terminal: {terminal!r}")
        stable: list[dict[str, Any]] = []
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            stable.append(health_snapshot(await runtime.health()))
            if len(stable) >= 3 and stable[-3:] == [stable[-1]] * 3:
                break
            await asyncio.sleep(0.02)
        else:
            raise AssertionError("runtime health did not stabilize after submit/cancel")
        health_after = stable[-1]
        if health_after["task_count"] != health_before["task_count"]:
            raise AssertionError("submit/cancel changed steady-state runtime task count")
        return {
            "included_in_performance_comparison": False,
            "submitted": CANCEL_OPERATIONS,
            "cancel_found": found,
            "cancel_queued": sum(report.queued for report in reports),
            "cancel_in_flight": sum(report.in_flight for report in reports),
            "terminal_kinds": terminal_kinds,
            "health_before": health_before,
            "health_stabilization_samples": stable,
        }
    finally:
        await runtime.stop()
        stopped_health = health_snapshot(await runtime.health())
        if stopped_health["task_count"] != 0 or stopped_health["accepting_commands"]:
            raise AssertionError(f"runtime did not stop cleanly: {stopped_health!r}")
        await asyncio.sleep(0.05)
        process_after = resource_snapshot()
        cleanup = assert_fd_cleanup(process_before, process_after, "submit-cancel-runtime")
        # Make cleanup evidence available even when a later assertion fails.
        submit_cancel_correctness_arm.cleanup = {"stopped_health": stopped_health, "process": cleanup}  # type: ignore[attr-defined]


async def measure_client_arms() -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
    process_before = resource_snapshot()
    async with bacnet.BACnetClient(
        interface=INTERFACE,
        port=PORT,
        broadcast_address=BROADCAST,
        apdu_timeout_ms=1_000,
    ) as client:
        chatty = await measure(lambda: chatty_workflow(client))
        gathered = await measure(lambda: chatty_workflow(client, True))
    await asyncio.sleep(0.05)
    process_after = resource_snapshot()
    return chatty, gathered, assert_fd_cleanup(process_before, process_after, "client-arms")


async def measure_runtime_arm() -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
    process_before = resource_snapshot()
    runtime = await start_discovered_runtime()
    health_before = health_snapshot(await runtime.health())
    try:
        crossings_before = runtime.crossing_counters()
        coarse = await measure(lambda: runtime_workflow(runtime))
        crossings_after = runtime.crossing_counters()
        crossing_delta = {
            "calls": crossings_after.calls - crossings_before.calls,
            "input_items": crossings_after.input_items - crossings_before.input_items,
            "output_items": crossings_after.output_items - crossings_before.output_items,
        }
        expected_calls = WARMUPS + SAMPLES
        expected_crossings = {"calls": expected_calls, "input_items": expected_calls * 6, "output_items": expected_calls * 6}
        if crossing_delta != expected_crossings:
            raise AssertionError(f"unexpected crossing delta: {crossing_delta!r}")
        health_after = health_snapshot(await runtime.health())
        if health_after["attachment_error_codes"] != [None] or health_after["task_count"] != health_before["task_count"]:
            raise AssertionError(f"unhealthy benchmark runtime: {health_after!r}")
    finally:
        await runtime.stop()
    stopped_health = health_snapshot(await runtime.health())
    if stopped_health["task_count"] != 0 or stopped_health["accepting_commands"]:
        raise AssertionError(f"runtime did not stop cleanly: {stopped_health!r}")
    await asyncio.sleep(0.05)
    process_after = resource_snapshot()
    cleanup = assert_fd_cleanup(process_before, process_after, "coarse-runtime")
    return coarse, crossing_delta, {
        "health_before": health_before,
        "health_after": health_after,
        "stopped_health": stopped_health,
        "process": cleanup,
    }


def identities() -> dict[str, Any]:
    wheel_fields = WHEEL_DIGEST_FILE.read_text().split()
    if len(wheel_fields) < 2 or len(wheel_fields[0]) != 64:
        raise AssertionError("installed wheel provenance is missing or malformed")
    build_environment = parse_key_values(BUILD_ENVIRONMENT_FILE)
    source = {
        "context": os.environ["BACNET_BENCH_SOURCE_CONTEXT"],
        "sha": os.environ["BACNET_BENCH_SOURCE_SHA"],
        "ref": os.environ["BACNET_BENCH_SOURCE_REF"],
        "dirty": os.environ["BACNET_BENCH_SOURCE_DIRTY"] == "true",
        "archive_sha256": os.environ["BACNET_BENCH_SOURCE_ARCHIVE_SHA256"],
    }
    for key in ("source_sha", "source_ref", "source_archive_sha256", "source_dirty"):
        if build_environment[key] != str(source[key.removeprefix("source_")]).lower():
            raise AssertionError(f"build/source identity mismatch for {key}")
    return {
        "source": source,
        "wheel": {"filename": wheel_fields[1].rsplit("/", 1)[-1], "sha256": wheel_fields[0]},
        "harness": {"path": str(HARNESS_FILE), "sha256": hashlib.sha256(HARNESS_FILE.read_bytes()).hexdigest()},
        "images": {"base": os.environ["BACNET_BENCH_BASE_IMAGE_ID"], "benchmark": os.environ["BACNET_BENCH_IMAGE_ID"]},
        "build_environment": build_environment,
    }


def write_result(result: dict[str, Any]) -> None:
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")


async def run_client() -> None:
    require_positive_configuration()
    result: dict[str, Any] = {
        "schema_version": 2,
        "suite": "python-runtime-ab",
        "scenario": "direct-bip-six-property-read-v2",
        "status": "running",
        "identities": identities(),
        "environment": {
            "python": sys.version,
            "python_executable": sys.executable,
            "rusty_bacnet_module": str(Path(bacnet.__file__).resolve()),
            "platform": platform.platform(),
            "machine": platform.machine(),
            "cpu_count": os.cpu_count(),
        },
        "configuration": {
            "interface": INTERFACE,
            "broadcast": BROADCAST,
            "port": PORT,
            "device_instance": DEVICE_INSTANCE,
            "server_address": SERVER_ADDRESS,
            "warmups": WARMUPS,
            "samples": SAMPLES,
            "trials": TRIALS,
            "cancel_operations": CANCEL_OPERATIONS,
            "rss_sample_interval_ms": int(RSS_INTERVAL * 1_000),
            "alternating_trial_design": True,
        },
        "correctness": {"fixture_values_checked_per_workflow": True},
        "trials": [],
    }
    result["process_before_stabilization"] = resource_snapshot()
    write_result(result)
    try:
        result["correctness"]["native_runtime_stabilization"] = await stabilize_native_runtime()
        write_result(result)

        for trial in range(TRIALS):
            if trial % 2 == 0:
                chatty, gathered, client_cleanup = await measure_client_arms()
                coarse, crossing_delta, runtime_cleanup = await measure_runtime_arm()
                arm_order = ["chatty", "gathered", "coarse"]
            else:
                coarse, crossing_delta, runtime_cleanup = await measure_runtime_arm()
                chatty, gathered, client_cleanup = await measure_client_arms()
                arm_order = ["coarse", "chatty", "gathered"]
            result["trials"].append({
                "trial": trial,
                "arm_order": arm_order,
                "chatty": {**chatty, "python_calls_per_workflow": 6},
                "gathered": {**gathered, "python_calls_per_workflow": 6},
                "coarse": {**coarse, "python_calls_per_workflow": 1},
                "crossings": crossing_delta,
                "cleanup": {"client": client_cleanup, "runtime": runtime_cleanup},
            })
            write_result(result)

        # Cancellation deliberately perturbs the server with work that may
        # already be on the wire. Exercise it only after the timed trials so
        # server-side cleanup cannot contaminate a later measurement arm.
        submit_cancel = await submit_cancel_correctness_arm()
        submit_cancel["cleanup"] = submit_cancel_correctness_arm.cleanup  # type: ignore[attr-defined]
        result["correctness"]["submit_cancel_stabilization"] = submit_cancel

        coarse_rates = [trial["coarse"]["workflows_per_second"] for trial in result["trials"]]
        chatty_rates = [trial["chatty"]["workflows_per_second"] for trial in result["trials"]]
        coarse_cv = statistics.pstdev(coarse_rates) / statistics.mean(coarse_rates)
        chatty_cv = statistics.pstdev(chatty_rates) / statistics.mean(chatty_rates)
        result["aggregate"] = {
            "coarse_throughput_cv": coarse_cv,
            "chatty_throughput_cv": chatty_cv,
            "median_coarse_workflows_per_second": statistics.median(coarse_rates),
            "median_chatty_workflows_per_second": statistics.median(chatty_rates),
            "median_speedup": statistics.median(coarse / chatty for coarse, chatty in zip(coarse_rates, chatty_rates)),
        }
        result["status"] = "complete" if coarse_cv <= 0.05 and chatty_cv <= 0.05 else "inconclusive-variance"
        result["process_after_trials"] = resource_snapshot()
        write_result(result)
        print(json.dumps({"status": result["status"], **result["aggregate"]}, sort_keys=True))
        if result["status"] != "complete":
            raise AssertionError("benchmark throughput CV exceeded 5%; result is inconclusive")
    except BaseException as error:
        if result["status"] == "running":
            result["status"] = "failed-correctness-or-cleanup"
        result["failure"] = {"type": type(error).__name__, "message": str(error)}
        result["process_at_failure"] = resource_snapshot()
        write_result(result)
        raise


def main() -> None:
    if len(sys.argv) != 2 or sys.argv[1] not in {"server", "client"}:
        raise SystemExit("usage: bench_runtime_batch_ab.py {server|client}")
    asyncio.run(run_server() if sys.argv[1] == "server" else run_client())


if __name__ == "__main__":
    main()
