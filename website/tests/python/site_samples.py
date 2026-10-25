"""The code samples the website displays, and how each one is verified.

Every fenced block and every Starlight ``<Code>`` component under
``src/content/docs`` must have exactly one entry in ``SAMPLES``; a new block
without one, or an entry whose block is gone, fails ``test_site_samples``.
An entry is keyed by the page and the block's first line (for a component,
its tag line), so moving a block within its page needs no change here, but
editing its first line does.

The classes match section 6 of ``docs/design/website-audit-0.15.md``:

- ``Run``: CI runs each ``bacnet`` command against a loopback server and
  checks its exit status and output (class A).
- ``Covered``: another CI test runs or compiles this block (class A).
- ``Parse``: CI runs each ``bacnet`` command with ``--help`` appended, so clap
  checks every subcommand, option and value before it; nothing is sent
  (class D). Used for samples that would leave the host.
- ``Skip``: not run by CI, with the audit class and the reason.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
import re
import shlex

WEBSITE = Path(__file__).resolve().parents[2]
DOCS = WEBSITE / "src/content/docs"


@dataclass(frozen=True)
class Expect:
    """One command's outcome: its exit status and patterns its output must match."""

    status: int = 0
    patterns: tuple[str, ...] = ()
    # A command that runs until stopped (subscribe): wait for the patterns,
    # then end it with SIGTERM, as the page's Ctrl+C would.
    stop: bool = False
    # A line the server must print after this command, before the next runs.
    server_line: str = ""


def ok(*patterns: str) -> Expect:
    return Expect(0, patterns)


@dataclass(frozen=True)
class Run:
    """Run against ``server`` ("lab", "server_device" or "none"); a word
    naming ``device``, alone or with a port, becomes the server's address."""

    device: str
    expect: tuple[Expect, ...]
    server: str = "lab"


@dataclass(frozen=True)
class Covered:
    # The test files, relative to the repository root.
    by: tuple[str, ...]


@dataclass(frozen=True)
class Parse:
    # Words allowed before `bacnet` on a line, such as `ssh` and its operands.
    wrapper: tuple[str, ...] = ()
    # Uses an option the CLI accepts only on Linux; elsewhere clap refuses it,
    # as the page says, and the check expects that refusal.
    linux_only: bool = False


@dataclass(frozen=True)
class Skip:
    cls: str
    reason: str


@dataclass(frozen=True)
class Block:
    page: str
    line: int
    kind: str  # "fence" or "component"
    lang: str
    first: str
    text: str = field(repr=False)

    @property
    def where(self) -> str:
        return f"{self.page}:{self.line}"


def blocks(docs: Path = DOCS) -> list[Block]:
    """Every fenced block and ``<Code>`` component, in page order."""
    found = []
    for path in sorted(docs.rglob("*.md*")):
        page = path.relative_to(docs).as_posix()
        lines = path.read_text(encoding="utf-8").splitlines()
        fence = None
        for number, line in enumerate(lines, 1):
            stripped = line.strip()
            if fence is not None:
                if stripped == "```":
                    body = fence[2]
                    first = next((item.strip() for item in body if item.strip()), "")
                    found.append(Block(page, fence[0], "fence", fence[1], first, "\n".join(body)))
                    fence = None
                else:
                    fence[2].append(line)
            elif stripped.startswith("```"):
                fence = (number, stripped[3:].strip(), [])
            elif stripped.startswith("<Code "):
                found.append(Block(page, number, "component", "", stripped, stripped))
        if fence is not None:
            raise ValueError(f"{page}:{fence[0]}: unclosed code fence")
    return found


def key(block: Block) -> str:
    return f"{block.page}::{block.first}"


def commands(text: str) -> list[list[str]]:
    """The shell commands in a block, split into words: continuations joined,
    blank lines and comments dropped."""
    joined = re.sub(r"\\\n", " ", text)
    out = []
    for line in joined.splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            out.append(shlex.split(line))
    return out


DEVICE = "192.168.1.100"
IT = "crates/bacnet-integration-tests/tests"
FIRST_READ_A = Covered((f"{IT}/first_read_source.rs", f"{IT}/first_read.rs"))
SHARED_ENDPOINT_A = Covered((f"{IT}/shared_endpoint_source.rs", f"{IT}/shared_endpoint.rs"))
SERVER_A = Covered((f"{IT}/server_device_source.rs", f"{IT}/server_device.rs"))
SERVER_PY_A = Covered(("crates/rusty-bacnet/tests/test_website_server.py",))
ROUTER_A = Covered((f"{IT}/bip_router_source.rs", f"{IT}/bip_router_guide.rs"))
ROUTER_PY_A = Covered(("crates/rusty-bacnet/tests/test_website_router.py",))
POLICY_A = Covered((f"{IT}/sc_operator_policy_source.rs", f"{IT}/sc_operator_policy.rs"))
LAB_A = Covered(("website/tests/python/test_loopback_lab.py",))
PY_FIRST_READ_A = Covered(("crates/rusty-bacnet/tests/test_website_first_read.py",))

INSTALL = "installs or builds software"
POWERSHELL = "PowerShell; the job that runs this test is Linux only"
NEEDS_PEERS = "needs BBMD, multicast or IPv6 peers on a real network"
NOT_HOST = "would send to a host other than loopback"
CLI_DISCOVER = ok(r'"instance": 1234', r'"address": "127\.0\.0\.1:\d+"')
REFUSED = Expect(4, ("allow-write-required", "nothing was sent"))
WRITTEN = ok(r'"status": "ok"')

# Samples, by page and first line. Keep them in page order.
SAMPLES: dict[str, object] = {
    "connect/across-subnets.mdx::bacnet bdt 192.168.1.10": Parse(),
    "connect/across-subnets.mdx::bacnet --foreign-bbmd 192.168.1.10 discover": Parse(),
    "connect/across-subnets.mdx::bacnet register 192.168.1.10 --ttl 300": Parse(),
    "connect/across-subnets.mdx::import asyncio": Skip("F", NEEDS_PEERS),
    "connect/across-subnets.mdx::from rusty_bacnet import BACnetServer, Bbmd": Skip("F", NEEDS_PEERS),
    "connect/bacnet-ip.md::bacnet --port 47810 read 192.168.1.100:47809 ai:1 pv":
        Run(DEVICE, (ok(r'"value": "72\.5"'),)),
    "connect/bacnet-sc.md::cargo install --path crates/bacnet-cli --locked --features sc-tls": Skip("F", INSTALL),
    "connect/bacnet-sc.md::bacnet --sc \\": Parse(),
    "connect/ipv6.mdx::bacnet --ipv6 --ipv6-interface 2001:db8::20 --device-instance 4321 read "
    "'[2001:db8::10]:47808' dev:1234 object-name": Parse(),
    "connect/ipv6.mdx::import asyncio": Skip("F", NEEDS_PEERS),
    "connect/mstp-realtime.mdx::cat /sys/bus/usb-serial/devices/ttyUSB0/latency_timer":
        Skip("F", "changes the system: a USB serial adapter's latency timer"),
    "connect/mstp-realtime.mdx::bacnet --mstp /dev/ttyUSB0 --mstp-mac 3 --mstp-rt-priority 40 --mstp-cpus 3 discover":
        Parse(linux_only=True),
    "connect/mstp-realtime.mdx::bacnet tui --mstp-monitor /dev/ttyUSB1 --mstp-baud 38400": Parse(),
    "connect/mstp-realtime.mdx::cargo run --release -p bacnet-benchmarks --bin stress-test -- mstp "
    "--reply small --steps 38400 --duration 5": Skip("F", "a release build of the benchmarks"),
    "connect/mstp.mdx::python examples/python/mstp_mini_device.py \\":
        Skip("D", "needs a serial port; the audit ran its argparse --help"),
    "connect/multicast.mdx::bacnet --interface 192.0.2.20 --bip-multicast 239.255.66.1 discover": Parse(),
    "connect/multicast.mdx::bacnet --interface 127.0.0.1 --port 0 --bip-multicast 239.255.66.1 "
    "--bip-multicast-port 47809 discover": Parse(),
    "connect/multicast.mdx::import asyncio": Skip("F", NEEDS_PEERS),
    "connect/multicast.mdx::from rusty_bacnet import BACnetServer, Bbmd, Multicast": Skip("F", NEEDS_PEERS),
    "connect/nat.mdx::from rusty_bacnet import Bbmd": Skip("F", NEEDS_PEERS),
    "connect/nat.mdx::bacnet --foreign-bbmd 203.0.113.1:47809 --ttl 600 --renewal 20 discover": Parse(),
    "development/build-from-source.md::git clone --branch dev https://github.com/jscott3201/rusty-bacnet.git":
        Skip("F", INSTALL),
    "development/build-from-source.md::cargo install --path crates/bacnet-cli --locked": Skip("F", INSTALL),
    "development/build-from-source.md::python3 -m venv .venv": Skip("F", INSTALL),
    "guides/discovery.mdx::bacnet discover --target 192.168.1.100 --wait 3": Run(DEVICE, (CLI_DISCOVER,)),
    "guides/discovery.mdx::bacnet discover 1000-2000 --wait 3": Parse(),
    "guides/discovery.mdx::bacnet --foreign-bbmd 192.168.1.1 --ttl 300 discover --wait 3": Parse(),
    'guides/discovery.mdx::bacnet find --name "Zone Temp" --wait 3': Parse(),
    "guides/observe-changes.md::bacnet subscribe 192.168.1.100 ai:1 --lifetime 300": Run(DEVICE, (
        Expect(0, ("Subscribed to ANALOG_INPUT:1", r'"type":"cov"', r'"PRESENT_VALUE":"72\.5"'), stop=True),
    )),
    "guides/safe-writes.md::bacnet readm 192.168.1.100 av:1 object-name,units,pv": Run(DEVICE, (
        ok("Zone Setpoint", r"enumerated\(64\)", "PRESENT_VALUE"),
        ok("PRIORITY_ARRAY", r'"value": "null(, null){15}"'),
        ok("RELINQUISH_DEFAULT"),
    )),
    # Without --allow-write the CLI refuses and sends nothing, so the value stays.
    "guides/safe-writes.md::bacnet write 192.168.1.100 av:1 pv 72.5 --priority 8":
        Run(DEVICE, (REFUSED, ok(r'"value": "0"'))),
    "guides/safe-writes.md::bacnet write 192.168.1.100 av:1 pv null --priority 8":
        Run(DEVICE, (REFUSED, ok(r'"value": "0"'))),
    "guides/safe-writes.md::bacnet --allow-write write 192.168.1.100 av:1 pv 72.5 --priority 8":
        Run(DEVICE, (WRITTEN, WRITTEN)),
    "integrate/router.mdx::bacnet whois-router": Parse(),
    'integrate/router.mdx::<Code code={routerManifest} lang="toml" title="Cargo.toml" />': ROUTER_A,
    'integrate/router.mdx::<Code code={routerSource} lang="rust" title="src/main.rs" />': ROUTER_A,
    'integrate/router.mdx::<Code code={routerPython} lang="python" title="bip_router.py" />': ROUTER_PY_A,
    "integrate/router.mdx::Router started: networks 1 and 5": ROUTER_PY_A,
    'integrate/security.mdx::<Code code={policyManifest} lang="toml" title="Cargo.toml" />': POLICY_A,
    'integrate/security.mdx::<Code code={policySource} lang="rust" title="src/main.rs" />': POLICY_A,
    "integrate/security.mdx::BACNET_DCC_PASSWORD='replace-me' cargo run -- operator.example.com": POLICY_A,
    "integrate/security.mdx::DCC without a certificate: denied": POLICY_A,
    'integrate/server.mdx::<Code code={serverManifest} lang="toml" title="Cargo.toml" />': SERVER_A,
    'integrate/server.mdx::<Code code={serverSource} lang="rust" title="src/main.rs" />': SERVER_A,
    "integrate/server.mdx::cargo run": SERVER_A,
    "integrate/server.mdx::.venv/bin/python server_device.py": SERVER_PY_A,
    'integrate/server.mdx::<Code code={serverPython} lang="python" title="server_device.py" />': SERVER_PY_A,
    "integrate/server.mdx::Serving device 1001 at 127.0.0.1:<assigned-port>": SERVER_PY_A,
    # The page's own program is the server; each write waits for the line the
    # program prints when it acts, as a reader watching its terminal would.
    "integrate/server.mdx::bacnet --interface 127.0.0.1 --port 0 read 127.0.0.1:<assigned-port> ai:1 pv": Run(
        "127.0.0.1:<assigned-port>",
        (
            ok(r'"value": "21\.5"'),
            Expect(0, (r'"status": "ok"',), server_line="Supply fan: on"),
            ok(r'"value": "19"'),
            Expect(0, (r'"status": "ok"',), server_line="Supply fan: off"),
        ),
        server="server_device",
    ),
    "integrate/shared-endpoints.mdx::shared-endpoint-guide/": Skip("G", "a directory tree"),
    'integrate/shared-endpoints.mdx::<Code code={endpointManifest} lang="toml" title="Cargo.toml" />':
        SHARED_ENDPOINT_A,
    'integrate/shared-endpoints.mdx::<Code code={endpointSource} lang="rust" title="src/main.rs" />':
        SHARED_ENDPOINT_A,
    "integrate/shared-endpoints.mdx::cargo run": SHARED_ENDPOINT_A,
    "integrate/shared-endpoints.mdx::Started endpoint: 127.0.0.1:<assigned-port>": SHARED_ENDPOINT_A,
    "project/performance.mdx::cargo bench -p bacnet-benchmarks --bench bip_latency":
        Skip("F", "benchmarks; run sequentially by hand"),
    "project/upgrading/v0-14.md::python -m pip install --upgrade rusty-bacnet==0.14.0": Skip("F", INSTALL),
    "start/embedded.md::cargo run -p bacnet-mstp-core --example node":
        Skip("C", "run in the audit; CI's all-targets clippy builds both examples"),
    "start/first-read.md::bacnet --version": Run("", (ok(r"^bacnet \d+\.\d+\.\d+"), ok("Usage: bacnet")), server="none"),
    "start/first-read.md::bacnet discover --target 192.168.1.100 --wait 3": Run(DEVICE, (CLI_DISCOVER,)),
    "start/first-read.md::bacnet read 192.168.1.100 device:1234 object-name":
        Run(DEVICE, (ok("OBJECT_NAME", "Local documentation lab"),)),
    "start/first-read.md::bacnet read 192.168.1.100 ai:1 pv": Run(DEVICE, (
        ok(r'"value": "72\.5"'),
        ok(r'"value": "72\.5"', r'"value": "enumerated\(64\)"', "Zone Temperature"),
    )),
    "start/first-read.md::bacnet --json read 192.168.1.100 ai:1 pv":
        Run(DEVICE, (ok(r'"property": "PRESENT_VALUE"', r'"value": "72\.5"'),)),
    "start/installation.mdx::grep ' bacnet-macos-arm64$' SHA256SUMS | shasum -a 256 -c":
        Skip("C", "needs the release download; run in the audit"),
    "start/installation.mdx::grep ' bacnet-linux-amd64$' SHA256SUMS | sha256sum -c":
        Skip("F", "needs the release download"),
    "start/installation.mdx::$sum = Select-String -Path .\\SHA256SUMS -SimpleMatch -Pattern '  bacnet-windows-amd64.exe'":
        Skip("F", POWERSHELL),
    "start/installation.mdx::.\\bacnet-windows-amd64.exe --version": Skip("F", POWERSHELL),
    'start/installation.mdx::<Code lang="sh" code={`git clone --branch v${release} ${repository}.git '
    'rusty-bacnet-v${release}': Skip("F", INSTALL),
    'start/installation.mdx::<Code lang="sh" code={`cargo install bacnet-cli --version ${release} --locked`} />':
        Skip("F", INSTALL),
    "start/installation.mdx::cargo install --path crates/bacnet-cli --locked --features sc-tls": Skip("F", INSTALL),
    'start/installation.mdx::<Code lang="sh" code={`python3 --version': Skip("F", INSTALL),
    # The environment's interpreter stands in for .venv/bin/python.
    "start/installation.mdx::.venv/bin/python -c \"from rusty_bacnet import BACnetClient; "
    "from importlib.metadata import version; print(version('rusty-bacnet'))\"":
        Run("", (ok(r"^\d+\.\d+\.\d+$"),), server="none"),
    'start/installation.mdx::<Code lang="powershell" code={String.raw`py -3.13 -m venv .venv': Skip("F", POWERSHELL),
    "start/installation.mdx::.\\.venv\\Scripts\\python.exe -c \"from rusty_bacnet import BACnetClient; "
    "from importlib.metadata import version; print(version('rusty-bacnet'))\"": Skip("F", POWERSHELL),
    'start/installation.mdx::<Code lang="toml" code={`[dependencies]':
        Skip("E", "the same table as start/rust.md, which first_read_source.rs checks"),
    "start/local-lab.mdx::my-bacnet-lab/": Skip("G", "a directory tree"),
    "start/local-lab.mdx::.venv/bin/python loopback_lab.py": LAB_A,
    "start/local-lab.mdx::.\\.venv\\Scripts\\python.exe loopback_lab.py": Skip("F", POWERSHELL),
    "start/local-lab.mdx::Example server: 127.0.0.1:<assigned-port>": LAB_A,
    "start/local-lab.mdx::.venv/bin/python loopback_lab.py --serve --seconds 300": LAB_A,
    "start/local-lab.mdx::.\\.venv\\Scripts\\python.exe loopback_lab.py --serve --seconds 300": Skip("F", POWERSHELL),
    'start/local-lab.mdx::<Code code={labSource} lang="python" title="loopback_lab.py" />': LAB_A,
    "start/python.md::from __future__ import annotations": PY_FIRST_READ_A,
    "start/python.md::python first_read.py 192.168.1.100:47808 --instance 1": PY_FIRST_READ_A,
    "start/rust.md::cargo new first-read": Skip("F", INSTALL),
    "start/rust.md::[dependencies]": FIRST_READ_A,
    "start/rust.md::use std::net::{Ipv4Addr, SocketAddrV4};": FIRST_READ_A,
    "start/rust.md::cargo run -- 192.168.1.100:47808": FIRST_READ_A,
    "tui/mstp.mdx::bacnet tui --mstp-monitor /dev/ttyUSB0 --mstp-baud 38400": Parse(),
    "tui/mstp.mdx::bacnet --mstp /dev/ttyUSB0 --mstp-mac 3 tui": Parse(),
    "tui/mstp.mdx::bacnet --mstp /dev/ttyUSB0 --mstp-mac 3 --mstp-rt-priority 40 --mstp-cpus 3 tui":
        Parse(linux_only=True),
    'tui/mstp.mdx::sudo usermod -aG dialout "$USER"': Skip("F", "changes the system: groups and a serial adapter"),
    'tui/overview.mdx::<Code lang="sh" code={`cargo install bacnet-cli --version ${release} --locked '
    '--features sc-tls,serial`} />': Skip("F", INSTALL),
    "tui/overview.mdx::bacnet": Parse(),
    "tui/profiles.mdx::bacnet tui --profile plant-a --passive": Parse(),
    "tui/profiles.mdx::BACNET_CONFIG_DIR=/srv/bacnet/site-a bacnet tui": Parse(),
    "tui/profiles.mdx::version = 1":
        Skip("E", "checked against the Profile, Bip and Safety serde types, which deny unknown fields"),
    "tui/troubleshooting.mdx::NO_COLOR=1 bacnet tui": Parse(),
    "tui/troubleshooting.mdx::ssh -t engineer@site-gateway bacnet tui --profile plant-a":
        Parse(wrapper=("ssh", "-t", "engineer@site-gateway")),
    "tui/watch.mdx:::watch 1001 ai:2": Skip("E", "TUI command-bar input, checked against command_bar.rs"),
    "tui/writes.mdx:::writem 1001 av:1 present-value 72@12 av:2 present-value 68@12":
        Skip("E", "TUI command-bar input, checked against command_bar.rs"),
    'tui/writes.mdx:::writem 1001 av:1 description "Zone 2 supply"@12':
        Skip("E", "TUI command-bar input, checked against command_bar.rs"),
}
