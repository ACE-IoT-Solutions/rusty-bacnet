"""Run release tools in a source workspace using the checkout's Rust version."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib


def release_env(checkout: Path) -> dict[str, str]:
    """Keep Cargo and rustc pinned when a source archive omits rust-toolchain.toml."""
    with (checkout / "rust-toolchain.toml").open("rb") as manifest:
        channel = tomllib.load(manifest)["toolchain"]["channel"]
    env = {**os.environ, "RUSTUP_TOOLCHAIN": channel}
    rustup = shutil.which("rustup")
    for tool in ("cargo", "rustc"):
        if rustup:
            # System binaries ahead of the rustup proxies ignore RUSTUP_TOOLCHAIN.
            executable = subprocess.check_output([rustup, "which", "--toolchain", channel, tool],
                                                 cwd=checkout, env=env, text=True).strip()
        else:
            executable = shutil.which(tool)
            if executable is None:
                raise FileNotFoundError(f"missing {tool} for release toolchain {channel}")
        env[tool.upper()] = executable
        version = subprocess.check_output([executable, "--version"], cwd=checkout, env=env, text=True)
        if version.split()[:2] != [tool, channel]:
            raise ValueError(f"expected {tool} {channel}, found {version.strip()}")
    # Maturin can invoke Cargo by name, including from the extracted workspace.
    env["PATH"] = os.pathsep.join((str(Path(env["CARGO"]).parent), env["PATH"]))
    return env


if __name__ == "__main__":
    workspace, *command = sys.argv[1:]
    sys.exit(subprocess.run(command, cwd=workspace, env=release_env(Path.cwd())).returncode)
