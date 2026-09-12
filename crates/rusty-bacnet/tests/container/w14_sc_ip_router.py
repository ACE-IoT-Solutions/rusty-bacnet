"""Run the installed-wheel SC-to-IP router acceptance and retain its result."""

from __future__ import annotations

import importlib.metadata
import json
import os
import platform
import shutil
import sys
import time
import unittest
from pathlib import Path


TEST_NAME = (
    "test_sc_ip_router.ScIpRouterTests."
    "test_routed_read_health_routes_and_hub_recovery"
)


def main() -> int:
    artifacts = Path(os.environ.get("W14_ARTIFACTS", "/artifacts"))
    artifacts.mkdir(parents=True, exist_ok=True)
    for name in ("build-environment.txt", "wheel.sha256"):
        source = Path("/usr/local/share/rusty-bacnet") / name
        if source.is_file():
            shutil.copy2(source, artifacts / name)

    started = time.time()
    suite = unittest.defaultTestLoader.loadTestsFromName(TEST_NAME)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    passed = result.wasSuccessful() and result.testsRun == 1 and not result.skipped
    record = {
        "schema_version": 1,
        "test": TEST_NAME,
        "tests_run": result.testsRun,
        "failures": len(result.failures),
        "errors": len(result.errors),
        "skipped": len(result.skipped),
        "passed": passed,
        "post_hub_restart_routed_read": passed,
        "started_unix_s": started,
        "finished_unix_s": time.time(),
        "python": sys.version,
        "platform": platform.platform(),
        "distribution": "rusty-bacnet",
        "distribution_version": importlib.metadata.version("rusty-bacnet"),
    }
    (artifacts / "w14-sc-ip-router-results.json").write_text(
        json.dumps(record, indent=2, sort_keys=True) + "\n"
    )
    if passed:
        print("W14_POST_HUB_RESTART_ROUTED_READ_PASS", flush=True)
        print("W14_SC_IP_ROUTER_ACCEPTANCE_PASS", flush=True)
        return 0
    print("W14_SC_IP_ROUTER_ACCEPTANCE_FAIL", flush=True)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
