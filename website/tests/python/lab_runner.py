"""Run the published lab against the same import supplied by an ACE test wheel."""
from importlib.metadata import version
from importlib.util import module_from_spec, spec_from_file_location
import os
from pathlib import Path
import sys

LAB = Path(__file__).resolve().parents[2] / "examples/python/loopback_lab.py"


def lab_command(*args):
    runner = Path(__file__) if os.environ.get("BACNET_LAB_EXPECTED_DISTRIBUTION") else LAB
    return [sys.executable, "-u", str(runner), *args]


def run():
    spec = spec_from_file_location("loopback_lab", LAB)
    lab = module_from_spec(spec)
    spec.loader.exec_module(lab)

    def wheel_version(name):
        if name != "rusty-bacnet":
            raise ValueError(f"Unexpected distribution in the published lab: {name}")
        return version(os.environ["BACNET_LAB_EXPECTED_DISTRIBUTION"])

    lab.version = wheel_version
    return lab.main()


if __name__ == "__main__":
    sys.exit(run())
