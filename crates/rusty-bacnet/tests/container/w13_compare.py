"""Normalize and compare W13 four-way results and retained module evidence."""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path


LEGS = (
    "bacpypes_to_bacpypes",
    "rusty_to_bacpypes",
    "bacpypes_to_rusty",
    "rusty_to_rusty",
)
MODULE_MARKERS = {
    "w1_discovery": "W13_MODULE_PASS module=w1_discovery",
    "w3_bbmd_fdt": "W3_BBMD_POLICY_ACCEPTANCE_PASS",
    "w5_routed_sources": "W5_VIRTUAL_ROUTER_ACCEPTANCE_PASS",
    "w6_cov_stream": "W6_COV_ACCEPTANCE_PASS",
    "w12_campus": "W12_ACCEPTANCE_PASS",
}


def canonical_properties(leg: dict) -> list[tuple[int, object, object]]:
    return [
        (item["property"], item["array_index"], item["value"])
        for item in leg["properties"]
    ]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> None:
    root = Path(sys.argv[1])
    legs = {name: json.loads((root / "legs" / f"{name}.json").read_text()) for name in LEGS}
    cleanup = json.loads((root / "cleanup-attestation.json").read_text())
    transcript_path = root / cleanup["transcript"]
    transcript = transcript_path.read_text()

    expected_inventory = [[2, index] for index in range(1, 221)]
    expected_object_list = sorted([[8, 4_171_300], *expected_inventory])
    expected_properties = [
        (77, None, "W13 Analog Value 001"),
        (85, None, 21.5),
        (117, None, 62),
        (28, None, "four-way parity"),
    ]
    expected_targets = {
        "bacpypes_to_bacpypes": "10.252.130.20:47822",
        "rusty_to_bacpypes": "10.252.130.20:47822",
        "bacpypes_to_rusty": "10.252.130.10:47821",
        "rusty_to_rusty": "10.252.130.10:47821",
    }
    normalized_object_lists = {}
    for name, leg in legs.items():
        leg["inventory"] = sorted(leg["inventory"])
        if leg["inventory"] != expected_inventory:
            raise AssertionError(f"{name}: normalized inventory mismatch")
        if leg["object_list"]["indexed_count"] != 221:
            raise AssertionError(f"{name}: Object_List count must be exactly 221")
        if leg["object_list"]["whole_error"] is not None:
            raise AssertionError(
                f"{name}: whole Object_List read failed: "
                f"{leg['object_list']['whole_error']}"
            )
        if leg["object_list"]["whole_count"] != leg["object_list"]["indexed_count"]:
            raise AssertionError(
                f"{name}: whole/indexed Object_List count mismatch: "
                f"{leg['object_list']['whole_count']} != "
                f"{leg['object_list']['indexed_count']}"
            )
        normalized_whole = sorted(leg["object_list"]["whole_values"])
        normalized_indexed = sorted(leg["object_list"]["indexed_values"])
        if normalized_whole != expected_object_list:
            raise AssertionError(f"{name}: whole Object_List membership mismatch")
        if normalized_indexed != expected_object_list:
            raise AssertionError(f"{name}: indexed Object_List membership mismatch")
        if normalized_whole != normalized_indexed:
            raise AssertionError(f"{name}: whole/indexed Object_List content mismatch")
        normalized_object_lists[name] = normalized_whole
        if leg["client_stack"] == "bacpypes3":
            if not leg["segmentation"]["outgoing_segmented_response_accepted"]:
                raise AssertionError(
                    f"{name}: client request did not advertise segmented response"
                )
            if not leg["segmentation"]["segmented_response_observed"]:
                raise AssertionError(f"{name}: no segmented ComplexACK was observed")
        else:
            for prefix in ("rpm", "auto", "batch_rp", "batch_rpm", "runtime"):
                if leg["object_list"][f"{prefix}_whole_count"] != leg["object_list"][
                    "indexed_count"
                ]:
                    raise AssertionError(f"{name}: {prefix} whole Object_List count mismatch")
                if sorted(leg["object_list"][f"{prefix}_whole_values"]) != expected_object_list:
                    raise AssertionError(f"{name}: {prefix} whole Object_List mismatch")
        if canonical_properties(leg) != expected_properties:
            raise AssertionError(f"{name}: exact property projection mismatch")
        if leg["errors"] != [
            {"category": "bacnet_error", "error_class": 1, "error_code": 31}
        ]:
            raise AssertionError(f"{name}: error normalization mismatch: {leg['errors']!r}")
        if len(leg["cov"]) != 1 or leg["cov"][0]["value"] != 22.5:
            raise AssertionError(f"{name}: COV mismatch: {leg['cov']!r}")
        if leg["cov"][0]["delivery"] != "confirmed":
            raise AssertionError(f"{name}: COV was not confirmed")
        if leg["client_stack"] == "bacpypes3" and not (
            leg["cov"][0]["confirmed_request_observed"]
            and leg["cov"][0]["simple_ack_observed"]
        ):
            raise AssertionError(f"{name}: confirmed COV request/ack was not observed")
        if leg["topology"] != {"target": expected_targets[name], "route": None}:
            raise AssertionError(f"{name}: unexpected topology projection")

    if any(value != expected_object_list for value in normalized_object_lists.values()):
        raise AssertionError("cross-leg Object_List mismatch")

    baseline = legs[LEGS[0]]
    baseline_properties = canonical_properties(baseline)
    for name in LEGS[1:]:
        if canonical_properties(legs[name]) != baseline_properties:
            raise AssertionError(
                f"{name}: property mismatch: {canonical_properties(legs[name])!r} "
                f"!= {baseline_properties!r}"
            )

    modules = {}
    for name, marker in MODULE_MARKERS.items():
        log = root / "modules" / f"{name}.log"
        text = log.read_text() if log.exists() else ""
        passed = marker in text
        modules[name] = {"passed": passed, "marker": marker, "log": str(log.relative_to(root))}
        if not modules[name]["passed"]:
            raise AssertionError(f"{name}: missing acceptance marker {marker!r}")

    if not (
        cleanup["scoped_containers_absent"]
        and cleanup["scoped_network_absent"]
        and cleanup["cleanup_marker"] == "W13_CLEANUP_OK"
        and "W13_CLEANUP_OK" in transcript
        and f"W13_CLEANUP_CONTAINERS_ABSENT project={cleanup['project']}" in transcript
        and f"W13_CLEANUP_NETWORK_ABSENT network={cleanup['project']}_matrix"
        in transcript
    ):
        raise AssertionError("cleanup attestation mismatch")
    for name in LEGS:
        if f"W13_LEG_PASS leg={name}" not in transcript:
            raise AssertionError(f"runner transcript missing leg {name}")
    for name in MODULE_MARKERS:
        if f"W13_MODULE_PASS module={name}" not in transcript:
            raise AssertionError(f"runner transcript missing module {name}")

    comparisons = {
        "inventory": {"equal": True, "analog_values": 220},
        "properties": {"equal": True, "projection": baseline_properties},
        "errors": {"equal": True, "category": "bacnet_error", "class": 1, "code": 31},
        "cov": {"equal": True, "delivery": "confirmed", "value": 22.5},
        "object_list": {
            "large": True,
            "whole_read_verified_all_legs": True,
            "segmented_response_observed_legs": [
                name for name in LEGS if legs[name]["client_stack"] == "bacpypes3"
            ],
            "rust_property_paths_verified": [
                "direct_rp",
                "direct_rpm",
                "auto_rp",
                "batch_rp",
                "batch_rpm",
                "runtime_read_batch",
            ],
            "indexed_fallback_verified": True,
            "count": baseline["object_list"]["whole_count"],
        },
        "topology": {
            "targets": expected_targets,
            "specialized_modules": ["w1_discovery", "w3_bbmd_fdt", "w5_routed_sources", "w12_campus"],
        },
    }

    hashes = {}
    for path in sorted(root.rglob("*")):
        if path.is_file() and path.name != "w13-cross-stack-results.json":
            hashes[str(path.relative_to(root))] = sha256(path)

    result = {
        "schema_version": 1,
        "status": "pass",
        "legs": legs,
        "semantic_comparisons": comparisons,
        "modules": modules,
        "cleanup": {
            "passed": True,
            "attestation": "cleanup-attestation.json",
            "transcript": cleanup["transcript"],
            "transcript_sha256": sha256(transcript_path),
        },
        "artifact_sha256": hashes,
    }
    output = root / "w13-cross-stack-results.json"
    output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(f"W13_RESULT path={output} sha256={sha256(output)}", flush=True)
    print("W13_SEMANTIC_COMPARISON_PASS legs=4 modules=5", flush=True)


if __name__ == "__main__":
    main()
