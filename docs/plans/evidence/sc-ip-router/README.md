# SC-to-IP router evidence

This directory retains evidence for the blocking R1-R4 implementation in
`docs/plans/sc-ip-router-workplan.md`.

Kickoff source pins:

- `main`: `9eed26a0126a980f0852bddef719bf71e537cc69`
- fetched `upstream/dev`: `b4c67ec6eafa1ce8b5fe1fa7c9241160460c30fc`

Focused pre-change baselines on macOS arm64:

- `cargo test -p bacnet-transport --locked --features sc-tls sc::`: 173 passed
- `cargo test -p bacnet-network --locked`: passed
- `cargo test -p bacnet-runtime --locked --features sc`: 80 passed
- `cargo check -p rusty-bacnet --tests --locked`: passed

Final local verification on macOS arm64:

- `cargo fmt --all -- --check`: passed.
- `cargo test -p bacnet-transport --locked --features sc-tls`: 676 unit tests,
  2 integration tests, 7 TLS tests, and 17 of 18 doc tests passed; one doc test
  is intentionally ignored.
- `cargo test -p bacnet-network --locked`: 90 passed.
- `cargo test -p bacnet-runtime --locked --features sc`: 81 passed.
- `cargo test -p bacnet-integration-tests --test sc_ip_router --locked`: 1
  passed.
- The existing shared `target/` initially exposed stale cross-crate metadata;
  `cargo clean -p bacnet-transport -p bacnet-network
  -p bacnet-integration-tests` repaired it. The final workspace proof used the
  stronger isolated target command `CARGO_TARGET_DIR=target/sc-ip-router-workspace-gate
  cargo test --workspace --exclude rusty-bacnet --locked --no-fail-fast
  --features bacnet-types/serde,bacnet-transport/ipv6,bacnet-transport/sc-tls`
  and passed. CI restores a Rust cache, so this is specifically fresh local
  target evidence rather than a claim about CI cache state.
- Fresh-target `cargo clippy --workspace --exclude rusty-bacnet --all-targets
  --locked`: passed with the repository's pre-existing warning set.
- CPython 3.13 installed-wheel suite, invoked outside the checkout: 152 tests
  and 1,872 subtests passed.
- W5 virtual-router container scenario: acceptance and restart passed; retained
  runner artifact directory was
  `/var/folders/pw/ms0yjfhd5ns58xt3d9fk_67r0000gn/T/rusty-bacnet-w5-router.WzPpSA`.
- `python3 scripts/generate-conformance-docs.py --check` and `git diff --check`:
  passed.

The passing W14 Linux arm64 artifact bundle is retained locally at
`target/w14-sc-ip-router-artifacts/`. Both the host source snapshot and the
wheel build record the matching SHA-256 digest
`dc25644ce1694f6c87f4a207fa10101fe522285c96e1d788adcabc6b7a2c46a2`.
The installed Linux arm64 wheel digest is
`a0922f20dfad1558fb52c034bce1bb7aee2fae3404140ad2ef654fc8d190b956`.
The runner recorded zero failures, zero errors, zero skips, a successful routed
read after the SC hub restart, and `W14_CLEANUP_OK`. Passing project-plan
acceptance tests does not by itself establish BACnet conformance.

Current source evidence:

- `crates/bacnet-transport/src/sc/reconnect.rs` and `sc/recovery.rs` implement
  the explicit unbounded policy and retry-eligibility stop condition.
- `crates/bacnet-network/src/router/tests/snapshots.rs` covers topology,
  health, and detached route snapshots.
- `crates/bacnet-network/src/router/tests/mixed_bip_sc.rs` covers unicast in
  both directions and B/IP-originated global broadcast over mixed ports.
- `crates/bacnet-integration-tests/tests/sc_ip_router.rs` covers a routed
  ReadProperty from B/IP to an SC server through the mutual-TLS Rust hub.
- `crates/rusty-bacnet/tests/test_router_ports.py` covers typed configuration,
  validation, snapshots, stop behavior, and restart for B/IP/virtual ports.
- `crates/rusty-bacnet/tests/test_sc_ip_router.py` covers installed-wheel source
  paths for routed SC traffic, health/routes, and hub-restart recovery.
- `crates/rusty-bacnet/tests/container/run-w14-sc-ip-router.sh` and its W14
  fixture files provide the container-scenario source.

The blocking R1-R4 execution proof is complete locally. Remaining nonblocking
work is oversize-NPDU rejection and counters (R7), re-announcement (R8), fork
wheel publication (R9), and multi-arch images (R10). CI execution remains a
delivery gate.
