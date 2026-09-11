# Container interoperability fixtures

These are explicit, non-CI Podman acceptance fixtures. Each runner builds and
installs a wheel before exercising it.

Run the W13/M7 four-way cross-stack acceptance harness with:

```sh
crates/rusty-bacnet/tests/container/run-w13-cross-stack.sh
```

The matrix runs bacpypes3→bacpypes3, rusty→bacpypes3, bacpypes3→rusty, and
rusty→rusty legs against equivalent large-object servers on non-default ports.
Every leg requires a successful 221-element whole Object_List read, then
independently verifies the indexed fallback and writes normalized inventory,
topology, property, error, and confirmed-COV JSON. Rust-client legs also verify
whole-list RPM and auto-routed RP; bacpypes3-client legs observe the actual
segmented ComplexACK. The pinned
bacpypes3 0.0.102 `NormalApplication` constructs its ASASAP before installing
the supplied Device object, so the fixture explicitly binds that object to the
ASASAP; per-leg evidence records that its confirmed requests advertised
segmented-response acceptance. The orchestrator then runs the existing
W1, W3, W5, W6, and W12 fixtures as bounded modules for duplicate observations,
BBMD/FDT expiry, byte-exact routed sources, sustained COV, and campus routing.
`w13-cross-stack-results.json` embeds every leg, semantic comparisons, module
status, and SHA-256 hashes for retained evidence. The runner prints its artifact
directory and verifies removal of its scoped containers and network. It is not
wired into CI.

Run the standards-correct W12 campus forwarding acceptance with:

```sh
crates/rusty-bacnet/tests/container/run-w12-campus.sh
```

The fixture uses two isolated IP broadcast domains joined by an explicit
containerized campus IP router. Before either BBMD starts, targeted discovery
must find no remote device. Peered BBMDs then carry the discovery broadcast;
their policy observations prove the Original-Broadcast and Forwarded-NPDU
sender/origin chain. Per Annex J, the routed confirmed request, unicast I-Am
reply, confirmed COV notification, and COV acknowledgements use the explicit
IP route to the remote B/IP router rather than a non-standard BBMD unicast
tunnel. The final device is on a process-local virtual data-link network with
no IP endpoint; a direct request to the router cannot reach it. Logs remain in
the printed `W12_CAMPUS_ARTIFACT_DIR`. This fixture is not wired into CI.

Run W5 Python-composable virtual-router interoperability acceptance with:

```sh
crates/rusty-bacnet/tests/container/run-w5-virtual-router.sh
```

One installed-wheel Python process hosts a fixed-address B/IP router and two
named virtual networks with one device each. A separate pinned
`bacpypes3==0.0.102` scanner discovers the router via IARTN, discovers each
routed device, reads both devices using exact one-byte DNET/DADR destinations,
and asserts byte-exact SNET/SADR source addresses. The host then stops and
restarts the same router, proving virtual membership and B/IP socket release;
the external scanner repeats IARTN, discovery, source-address, and read checks.
Logs remain in the printed `W5_VIRTUAL_ROUTER_ARTIFACT_DIR`. Set
`W5_SKIP_BUILD=1` to reuse both local fixture images.

Run W6 COV interoperability acceptance with:

```sh
crates/rusty-bacnet/tests/container/run-w6-cov.sh
```

The runner starts an installed-wheel Rust server and a pinned
`bacpypes3==0.0.102` subscriber on one isolated bridge. It validates analog
`covIncrement` filtering, any-change behavior for binary, multi-state, and
character-string values, confirmed and unconfirmed delivery, finite-lifetime
expiry, and exact count/order for a deterministic 1,000-write workload in each
delivery mode. Raw logs and a JSON record of every expected and observed event
remain in the printed `W6_COV_ARTIFACT_DIR`. Set `W6_SKIP_BUILD=1` to reuse both
local fixture images.

Run W3 BBMD actor/policy interoperability acceptance with:

```sh
crates/rusty-bacnet/tests/container/run-w3-bbmd-policy.sh
```

The runner creates two installed-wheel Rust BBMDs with reciprocal unicast BDT
entries and two pinned-bacpypes3 foreign devices on distinct logical `/24`
subnets. A compose-owned `/16` carrier bridge preserves source addresses under
rootless Podman; the foreign-device adapter disables only bacpypes3's unused
local-broadcast listener, while retaining its real BIPForeign registration and
BVLL stack. The fixture verifies cross-subnet traffic in both directions,
exact-once cut-through delivery, policy drop and Register-FD NAK behavior,
native and Python policy counters, and actor-observable FDT expiry after the
30-second BACnet grace period. It verifies removal of its own containers and
network and is not wired into CI. Set `W3_SKIP_BUILD=1` to reuse both local
fixture images.
