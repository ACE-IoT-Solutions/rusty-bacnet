"""Run the website's CLI samples against loopback servers (#1989).

Every displayed block needs an entry in site_samples.SAMPLES. The ``Run``
samples run against the loopback lab (or the server guide's own program) on
127.0.0.1, with the page's device address replaced by the server's; any other
address, or a transport that could leave the host, is refused before a
command starts. The ``Parse`` samples run with ``--help`` appended.

The CLI comes from BACNET_CLI, or target/debug/bacnet when that exists; the
CLI tests skip without one, and fail if BACNET_CLI names a missing file.
Run from the repository root, in an environment with rusty-bacnet installed:
python -m unittest discover -s website/tests/python -v
"""
from __future__ import annotations

import ipaddress
import os
from pathlib import Path
import queue
import re
import signal
import subprocess
import sys
import threading
import time
import unittest

import site_samples as samples
from site_samples import Covered, Expect, Parse, Run, Skip

REPO = samples.WEBSITE.parent
LAB = samples.WEBSITE / "examples/python/loopback_lab.py"
SERVER_DEVICE = samples.WEBSITE / "examples/python/server_device.py"
LOOPBACK = "127.0.0.1"
TIMEOUT = 30
# Options that pick a transport or a peer that may be off the host.
OFF_HOST_OPTIONS = {
    "--sc", "--sc-url", "--sc-failover-url", "--mstp", "--mstp-monitor", "--ipv6", "--ipv6-interface",
    "--foreign-bbmd", "--bbmd", "--bip-multicast", "--dnet",
}
# The subcommands a run sample may use, and what names the peer: the first
# operand (a device address or instance), or the --target option.
TARGET_FIRST = {"read", "rp", "readm", "rpm", "write", "wp", "writem", "wpm", "subscribe", "cov"}
TARGET_OPTION = {"discover", "whois"}
# Global options a run sample may put before its subcommand; any other fails closed.
GLOBAL_FLAGS = {"--json", "--allow-write", "--yes", "--version", "-V", "--help", "-h"}
GLOBAL_VALUES = {"-i", "--interface", "-p", "--port", "-b", "--broadcast", "-t", "--timeout"}
LOOPBACK_PEER = re.compile(r"(?:(127\.\d{1,3}\.\d{1,3}\.\d{1,3})|\[(::1)\])(?::\d{1,5})?|\d+")
# Global options the harness adds when a sample doesn't set them, so the CLI
# binds and broadcasts on loopback only.
LOOPBACK_DEFAULTS = (("-i", "--interface", LOOPBACK), ("-p", "--port", "0"), ("-b", "--broadcast", LOOPBACK))
IPV4 = re.compile(r"(?<![\d.])(\d{1,3}(?:\.\d{1,3}){3})(?![\d.])")
HOSTNAME = re.compile(r"^[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}(?::\d+)?$")


class OffHost(ValueError):
    pass


def check_loopback(words: list[str]) -> None:
    """Refuse any word that names a peer off this host."""
    for word in words:
        name, _, inline = word.partition("=") if word.startswith("--") else (word, "", "")
        if name in OFF_HOST_OPTIONS:
            raise OffHost(f"{name} could reach another host")
        for text in filter(None, (word, inline)):
            if "://" in text or HOSTNAME.match(text):
                raise OffHost(f"{text!r} names a remote host")
            for match in IPV4.finditer(text):
                if not ipaddress.ip_address(match.group(1)).is_loopback:
                    raise OffHost(f"{match.group(1)} is not a loopback address")
            v6 = re.fullmatch(r"\[([0-9A-Fa-f:.]+)\](?::\d+)?|([0-9A-Fa-f]*:[0-9A-Fa-f:]*:[0-9A-Fa-f:.]*)", text)
            if v6:
                try:
                    loopback = ipaddress.ip_address(v6.group(1) or v6.group(2)).is_loopback
                except ValueError:
                    loopback = False
                if not loopback:
                    raise OffHost(f"{text!r} is not a loopback address")


def split_env(words: list[str]) -> tuple[dict[str, str], list[str]]:
    env = {}
    while words and re.fullmatch(r"[A-Z_][A-Z0-9_]*=.*", words[0]):
        name, _, value = words[0].partition("=")
        env[name] = value
        words = words[1:]
    return env, words


def cli_argv(cli: str, words: list[str], device: str, address: str) -> list[str]:
    """The command to run for one sample line: loopback defaults added, the
    device address replaced, then checked."""
    if words[0] != "bacnet":
        raise ValueError(f"not a CLI command: {words}")
    rest = []
    for word in words[1:]:
        if device and (word == device or word.startswith(device + ":")):
            word = address
        rest.append(word)
    added = []
    for short, long, value in LOOPBACK_DEFAULTS:
        if not any(w in (short, long) or w.startswith(long + "=") for w in rest):
            added += [long, value]
    check_loopback(added + rest)
    check_peer(added + rest)
    return [cli, *added, *rest]


def check_peer(words: list[str]) -> None:
    """The word naming the peer must be a loopback literal or a device
    instance (found by a Who-Is on the loopback broadcast address). This
    closes the gap check_loopback leaves for single-label host names, which
    can't be told from an operand elsewhere on the line."""
    index = 0
    while index < len(words) and words[index].startswith("-"):
        name = words[index].split("=", 1)[0]
        if name in GLOBAL_VALUES and "=" not in words[index]:
            index += 2
        elif name in GLOBAL_FLAGS or name in GLOBAL_VALUES:
            index += 1
        else:
            raise OffHost(f"{words[index]} before the subcommand is not allowed in a run sample")
    if index >= len(words):
        return  # no subcommand: --version or --help
    command, operands = words[index], words[index + 1:]
    if command in TARGET_FIRST:
        peer = operands[0] if operands else ""
    elif command in TARGET_OPTION:
        peers = [operands[i + 1] for i, w in enumerate(operands[:-1]) if w == "--target"]
        peers += [w.split("=", 1)[1] for w in operands if w.startswith("--target=")]
        if not peers:
            raise OffHost(f"{command} without --target would broadcast")
        peer = peers[0]
    else:
        raise OffHost(f"{command} is not a subcommand run samples may use")
    match = LOOPBACK_PEER.fullmatch(peer)
    if not match or (match.group(1) and not ipaddress.ip_address(match.group(1)).is_loopback):
        raise OffHost(f"{peer!r} is not a loopback peer")


def parse_argv(cli: str, words: list[str], spec: Parse) -> list[str]:
    """The command for a parse check: the CLI's own words, then --help."""
    start = words.index("bacnet") if "bacnet" in words else -1
    if start < 0 or tuple(words[:start]) != spec.wrapper:
        raise ValueError(f"not a CLI command (wrapper {spec.wrapper}): {words}")
    return [cli, *words[start + 1:], "--help"]


def find_cli() -> str | None:
    named = os.environ.get("BACNET_CLI")
    if named:
        if not Path(named).is_file():
            raise FileNotFoundError(f"BACNET_CLI={named} does not exist")
        return named
    built = REPO / "target/debug" / ("bacnet.exe" if os.name == "nt" else "bacnet")
    return str(built) if built.is_file() else None


def text(output: str | bytes | None) -> str:
    if isinstance(output, bytes):
        output = output.decode(errors="replace")
    return (output or "").replace("\r\n", "\n")


class Server:
    """A server program on loopback; its output lines are collected as it runs."""

    def __init__(self, argv: list[str], ready: str, stop_on_eof: bool):
        self.stop_on_eof = stop_on_eof
        self.lines: list[str] = []
        self.changed = threading.Condition()
        self.process = subprocess.Popen(
            argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        )
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()
        first = self.wait_for(ready)
        self.address = re.search(r"127\.0\.0\.1:\d+", first).group(0)
        check_loopback([self.address])

    def _read(self) -> None:
        for line in self.process.stdout:
            with self.changed:
                self.lines.append(text(line).rstrip("\n"))
                self.changed.notify_all()

    def wait_for(self, prefix: str, timeout: float = 15) -> str:
        deadline = time.monotonic() + timeout
        with self.changed:
            while True:
                found = next((line for line in self.lines if line.startswith(prefix)), None)
                if found is not None:
                    return found
                left = deadline - time.monotonic()
                if left <= 0 or self.process.poll() is not None:
                    raise AssertionError(f"server never printed {prefix!r}; output: {self.lines}")
                self.changed.wait(left)

    def stop(self) -> None:
        if self.process.poll() is None:
            if self.stop_on_eof:
                self.process.stdin.close()
            elif os.name != "nt":
                self.process.send_signal(signal.SIGINT)  # Ctrl+C: the lab stops its server
            else:
                self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.reader.join(timeout=5)
        for stream in (self.process.stdin, self.process.stdout):
            stream.close()


def start_server(kind: str) -> Server:
    if kind == "lab":
        return Server([sys.executable, "-u", str(LAB), "--serve", "--seconds", "600"], "Example server: ", False)
    if kind == "server_device":
        return Server([sys.executable, "-u", str(SERVER_DEVICE)], "Serving device 1001 at ", True)
    raise ValueError(kind)


def entries(kind: type) -> list[tuple[samples.Block, object]]:
    """The blocks with entries of this kind; fails if any such entry lost its
    block, so a sample edited out of its entry fails here, not only in the
    inventory test."""
    found = [(block, samples.SAMPLES[samples.key(block)]) for block in samples.blocks()
             if isinstance(samples.SAMPLES.get(samples.key(block)), kind)]
    keys = {samples.key(block) for block, _ in found}
    stale = [k for k, spec in samples.SAMPLES.items() if isinstance(spec, kind) and k not in keys]
    if stale:
        raise AssertionError(f"{kind.__name__} entries with no block on the page: {stale}")
    return found


class SampleInventory(unittest.TestCase):
    def test_every_block_has_exactly_one_entry(self):
        found = {samples.key(block): block.where for block in samples.blocks()}
        missing = sorted(f"{where}: {k!r}" for k, where in found.items() if k not in samples.SAMPLES)
        stale = sorted(k for k in samples.SAMPLES if k not in found)
        self.assertEqual(missing, [], "blocks with no entry in site_samples.SAMPLES")
        self.assertEqual(stale, [], "entries whose block is gone")

    def test_keys_are_unique(self):
        keys = [samples.key(block) for block in samples.blocks()]
        self.assertEqual(len(keys), len(set(keys)), "two blocks on a page share a first line")

    def test_entries_are_well_formed(self):
        for key, spec in samples.SAMPLES.items():
            with self.subTest(key=key):
                self.assertIsInstance(spec, (Run, Covered, Parse, Skip))
                if isinstance(spec, Skip):
                    self.assertIn(spec.cls, "CDEFG")
                    self.assertTrue(spec.reason)
                if isinstance(spec, Covered):
                    for path in spec.by:
                        self.assertTrue((REPO / path).is_file(), path)

    def test_run_and_parse_blocks_are_cli_lines(self):
        for block, spec in entries(Run) + entries(Parse):
            with self.subTest(block=block.where):
                lines = samples.commands(block.text)
                self.assertTrue(lines)
                if isinstance(spec, Run):
                    self.assertEqual(len(lines), len(spec.expect), "one expectation per command")
                    for words in lines:
                        self.assertIn(words[0], ("bacnet", ".venv/bin/python"))
                else:
                    for words in lines:
                        parse_argv("bacnet", split_env(words)[1], spec)


class LoopbackGuard(unittest.TestCase):
    def test_rewrites_the_device_and_adds_loopback_defaults(self):
        argv = cli_argv("cli", ["bacnet", "read", "192.168.1.100", "ai:1", "pv"], "192.168.1.100", "127.0.0.1:5")
        self.assertEqual(argv, ["cli", "--interface", LOOPBACK, "--port", "0", "--broadcast", LOOPBACK,
                                "read", "127.0.0.1:5", "ai:1", "pv"])
        argv = cli_argv("cli", ["bacnet", "--port", "47810", "read", "192.168.1.100:47809", "ai:1", "pv"],
                        "192.168.1.100", "127.0.0.1:5")
        self.assertEqual(argv[1:], ["--interface", LOOPBACK, "--broadcast", LOOPBACK,
                                    "--port", "47810", "read", "127.0.0.1:5", "ai:1", "pv"])

    def test_refuses_anything_off_host(self):
        for words in [
            ["bacnet", "read", "192.168.1.101", "ai:1", "pv"],
            ["bacnet", "--interface", "0.0.0.0", "read", "127.0.0.1:5", "ai:1", "pv"],
            ["bacnet", "--broadcast=192.168.1.255", "discover"],
            ["bacnet", "read", "[2001:db8::10]:47808", "ai:1", "pv"],
            ["bacnet", "read", "2001:db8::10", "ai:1", "pv"],
            ["bacnet", "read", "controller.example.com", "ai:1", "pv"],
            ["bacnet", "--sc-url", "wss://127.0.0.1/hub", "discover"],
            ["bacnet", "--foreign-bbmd", "127.0.0.1", "discover"],
            ["bacnet", "--bip-multicast", "239.255.66.1", "discover"],
            ["bacnet", "discover", "--dnet", "5"],
            ["bacnet", "--sc-failover-url", "wss://127.0.0.1/hub", "discover"],
            ["bacnet", "read", "controller", "ai:1", "pv"],
            ["bacnet", "discover", "--target", "controller"],
            ["bacnet", "discover", "1000-2000"],
            ["bacnet", "whois-router"],
            ["bacnet", "--format", "json", "read", "127.0.0.1:5", "ai:1", "pv"],
        ]:
            with self.subTest(words=words), self.assertRaises(OffHost):
                cli_argv("cli", words, "192.168.1.100", "127.0.0.1:5")

    def test_accepts_loopback(self):
        check_loopback(["127.0.0.1:47808", "[::1]:47808", "ai:1", "5:1005", "72.5", "object-name,units,pv"])
        for words in (["read", "[::1]:47808", "ai:1", "pv"], ["--json", "read", "1234", "ai:1", "pv"],
                      ["discover", "--target", "127.0.0.1:5", "--wait", "3"], ["--version"]):
            with self.subTest(words=words):
                check_peer(["--interface", LOOPBACK, "--port", "0", *words])


class CliSamples(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.cli = find_cli()
        if cls.cli is None:
            raise unittest.SkipTest("no CLI: set BACNET_CLI or build bacnet-cli")

    def run_line(self, words: list[str], spec: Run, address: str) -> tuple[list[str], dict[str, str]]:
        env, words = split_env(words)
        if words[0] == ".venv/bin/python":
            # Only a one-line `-c` program: a page's script must not run unguarded here.
            self.assertEqual((len(words), words[1]), (3, "-c"), words)
            argv = [sys.executable, *words[1:]]
        else:
            argv = cli_argv(self.cli, words, spec.device, address)
        environment = {**os.environ, **env}
        return argv, environment

    def check(self, block: samples.Block, spec: Run, server: Server | None) -> None:
        address = server.address if server else ""
        for words, expect in zip(samples.commands(block.text), spec.expect, strict=True):
            argv, env = self.run_line(words, spec, address)
            label = " ".join(argv[1:])
            if expect.stop:
                status, output = self.run_until(argv, env, expect)
            else:
                result = subprocess.run(argv, env=env, stdin=subprocess.DEVNULL, capture_output=True,
                                        text=True, timeout=TIMEOUT)
                status, output = result.returncode, text(result.stdout) + text(result.stderr)
            self.assertEqual(status, expect.status, f"{block.where}: {label}\n{output}")
            for pattern in expect.patterns:
                self.assertRegex(output, re.compile(pattern, re.M), f"{block.where}: {label}")
            if expect.server_line:
                server.wait_for(expect.server_line, timeout=5)

    def run_until(self, argv: list[str], env: dict[str, str], expect: Expect) -> tuple[int, str]:
        process = subprocess.Popen(argv, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, text=True)
        lines: queue.Queue[str] = queue.Queue()
        reader = threading.Thread(target=lambda: [lines.put(line) for line in process.stdout], daemon=True)
        reader.start()
        output, deadline = "", time.monotonic() + 15
        try:
            while not all(re.search(p, output, re.M) for p in expect.patterns) and time.monotonic() < deadline:
                try:
                    output += text(lines.get(timeout=0.2))
                except queue.Empty:
                    if process.poll() is not None:
                        break
        finally:
            if process.poll() is None:
                process.terminate()  # SIGTERM on POSIX, as the page's Ctrl+C or a service stop
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        reader.join(timeout=5)
        while not lines.empty():
            output += text(lines.get())
        process.stdout.close()
        return process.returncode, output

    def run_on(self, server_kind: str) -> None:
        chosen = [(block, spec) for block, spec in entries(Run) if spec.server == server_kind]
        self.assertTrue(chosen, f"no samples run on {server_kind}")
        server = None if server_kind == "none" else start_server(server_kind)
        try:
            for block, spec in chosen:
                with self.subTest(block=block.where):
                    self.check(block, spec, server)
        finally:
            if server:
                server.stop()

    def test_samples_without_a_server(self):
        self.run_on("none")

    def test_samples_on_the_loopback_lab(self):
        self.run_on("lab")

    def test_server_guide_samples_on_its_program(self):
        self.run_on("server_device")

    def test_parse_samples(self):
        for block, spec in entries(Parse):
            for words in samples.commands(block.text):
                env, words = split_env(words)
                argv = parse_argv(self.cli, words, spec)
                with self.subTest(block=block.where, command=" ".join(words)):
                    result = subprocess.run(argv, env={**os.environ, **env}, stdin=subprocess.DEVNULL,
                                            capture_output=True, text=True, timeout=TIMEOUT)
                    if spec.linux_only and not sys.platform.startswith("linux"):
                        self.assertEqual(result.returncode, 2, text(result.stdout))
                        self.assertIn("Linux only", text(result.stderr))
                        continue
                    self.assertEqual(result.returncode, 0, text(result.stderr))
                    self.assertIn("Usage: bacnet", text(result.stdout))


if __name__ == "__main__":
    unittest.main()
