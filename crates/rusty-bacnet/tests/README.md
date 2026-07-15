# Installed Python tests

These tests validate the packaged `rusty_bacnet` extension rather than Rust
internals. Use a clean virtual environment so a stale editable install cannot
hide a packaging or runtime/stub mismatch.

From the repository root:

```sh
python3.13 -m venv /tmp/rusty-bacnet-python-test
/tmp/rusty-bacnet-python-test/bin/python -m pip install --upgrade pip maturin
/tmp/rusty-bacnet-python-test/bin/maturin build \
  --manifest-path crates/rusty-bacnet/Cargo.toml \
  --interpreter /tmp/rusty-bacnet-python-test/bin/python \
  --out /tmp/rusty-bacnet-dist
/tmp/rusty-bacnet-python-test/bin/python -m pip install \
  /tmp/rusty-bacnet-dist/rusty_bacnet-*.whl
/tmp/rusty-bacnet-python-test/bin/python -m unittest discover \
  -s crates/rusty-bacnet/tests -v
```

The contract suite is dependency-free after the wheel is installed. Runtime
network and cross-stack fixtures will be added as later parity milestones land.

The B/IP loopback suite uses OS-assigned ports so it can run without privileged
network setup. Direct confirmed services and COV are fully asserted. Who-Is is
sent and its result type is checked, but deterministic I-Am receipt requires
distinct endpoints bound to the same BACnet UDP port; that assertion belongs in
the later container/network-topology fixture.

## Deterministic discovery fixture

The container fixture gives the installed-wheel client, Rust server, two raw
router responders, and a bacpypes3 0.0.102 BBMD distinct IPv4 addresses. It
asserts exact same-port Who-Is/I-Am addressing, scoped and unscoped router
claims, duplicate merging, Read-BDT/Read-FDT fields at non-default ports, and a
decreasing live FDT remaining time.

From the repository root:

```sh
crates/rusty-bacnet/tests/container/run-discovery.sh
```

Podman with `podman compose` (or `podman-compose`) is required. The runner always removes its
containers, network, and volumes on exit. This fixture is intentionally local
and is not wired into CI.
