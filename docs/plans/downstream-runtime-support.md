# Downstream BACnet runtime support

The published adapter baseline is `ace-rusty-bacnet==0.17.2`, tag `v0.17.2`
at `aa5c452364663155b77429729b2dd84f46f74b8f`. The current ACE 0.18.0
candidate combines downstream implementation and qualification through
`a3989363c` with upstream dev at `4ade57174`. Historical qualification
wheels described below carried version 0.17.2. Identify each wheel by its
source commit and SHA-256. The merged candidate builds version 0.18.0.

The current downstream contract is
`docs/reviews/bacnet-rusty-0172-parity-2026-11-25.md` at `ddeb861e` on
the local Edge branch `feature/rusty-bacnet-migration`. It supersedes the
November 24 review at `b718d9dc`. The Python reference at `39256f1` includes
local receive readiness and Linux socket reuse. Edge currently allows exactly
0.17.1 and 0.17.2 for experimental use. Adopting 0.18.0 requires an Edge
adapter update and validation of the revised UCS-4 value representation.

## Published 0.17.2

CI, Native and the build-only release workflow passed on the published source
commit. The release workflow built and smoke-tested artifacts on five platforms.
The source tag and public release are available on GitHub.
[Release v0.17.2](https://github.com/ACE-IoT-Solutions/rusty-bacnet/releases/tag/v0.17.2).
[CI](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/45644635261) ·
[Native](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/45644640939) ·
[Release build](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/45646121969)

All 11 files served by the ACE private index matched the staged SHA-256 hashes.
They comprise CPython 3.10 and 3.13 wheels for Linux x86_64 and aarch64, macOS
x86_64 and arm64, and Windows amd64, plus the source archive. Fresh installs
from that index passed smoke tests on macOS arm64 with both Python versions
and on Linux aarch64 with Python 3.13. The unpacked source archive passed
`cargo check --locked --offline -p rusty-bacnet --lib`.

| Published artifact used in these checks | SHA-256 |
| --- | --- |
| CPython 3.10, macOS arm64 | `9a85673e1314692a678e0876c5a023bd752d5643460bb1577a5d57ded044f8c9` |
| CPython 3.13, macOS arm64 | `1cdccecf619683015270b637ee28aced147d9754debe5adc3a53fb64e3b93ed8` |
| CPython 3.13, manylinux 2.17 aarch64 | `4a5b795ae65184b1088c6dc42f4df58892d5f1a80af4b2f4b3589bb4d41c4f73` |
| Source archive | `871569982588aa50b35ef84447f7b32f523338427890e6dd205f28b57b7ca895` |

The published manylinux aarch64 wheel also passed the segmented COV probe with
bacpypes3 0.0.106. The standalone and shared endpoint clients each received
13-segment requests, returned seven SegmentACKs per request, rejected the
unknown subscription and delivered the admitted notification with its full
6,000-character description. On the same wheel, UIDs 10001 and 10002 shared
UDP port 47808 on different loopback addresses in an isolated Linux namespace.
Both received broadcasts and returned their own Device identities. An occupied
address refused another bind, and restoring one endpoint after a failed
replacement left the other readable.

The local release records are under `dist/python/0.17.2/`.
`publication-manifest.json` and `published-index-verification.json` record
each filename, size and digest. `sdist-verification.json` records the offline
check, `release-readiness.json` records the gates and installation smokes, and
`linux-qualification/release-*.json` contains the COV and cross-UID traces.

### Protocol behavior in the published release

| Use | Contract and evidence |
| --- | --- |
| Segmented reads | Shared and standalone clients acknowledge segment zero before the peer sends the negotiated window. The bacpypes3 probe with 480 service bytes per segment returned equal values through both owners. RP used 13 segments and seven SegmentACKs, RPM used 26 and 14, and the largest response APDU was 485 bytes. |
| Trend and file reads | ReadRange and AtomicReadFile use the same receiver. The strict sender at `dc3e77de2` waits for ACK 0, 2 and 3. Tests preserve trend record bytes, flags and lenient sequence zero, and a file window at offset 3072 with 1,800 octets and EOF. |
| Receive health | `BipEndpoint.status()["transport_health"]` reports primary receive and required/active broadcast listeners independently of foreign registration. Tests at `57a523f73` cover listener loss with working unicast, recoverable input errors and restart. |
| Confirmed COV | A positive `confirmed_cov_subscription_limit` creates an empty bounded registry. Retain a source-qualified expectation lease before subscribing. Matching single-object notifications receive SimpleACK. Unknown subscriptions, including confirmed Multiple in strict mode, receive an Error and produce no event. Both incoming confirmed COV forms support bounded reassembly when the endpoint Device declares Receive or Both. The default Device declares None. Passive unconfirmed delivery remains available. |

A full worker queue drops excess receive work. Endpoint shutdown joins the
worker even when a Rust client handle survives. Replaying a whole request can
publish again. Process-ID allocation, expectations and subscription renewal
belong to the caller. Immediate reuse of the same wire identity can admit a
late notification. [Python API guide](../python-api.md)

#### Candidate qualification

Historical pre-release checks at `bcc0422c2` passed 4,520 macOS tests and 45
doctests, plus 4,524 Linux arm64 tests and 11 Linux health cases. The candidate
macOS wheel SHA-256 was
`14ce13bc0fe1dc62777ef3a41357bbf61239e82efa42ba1774b85ea0c6413f97`.
The 216-sample standalone instruction-count comparison against `dc3e77de2`
passed all six RP/RPM/WP rows with unchanged allocation counts. These records
are scoped to those revisions. Historical records include
`dist/segmentation-7b59ac33d-verification.json`, `dist/bcc0422c2/` and
`target/downstream-linux/perf-bcc0422c2/summary.json`.

## Native changes in 0.18.0

| Source | Contract available to downstream callers |
| --- | --- |
| `f90a6b430`, `35d9abb2f`, `ae9a86178` | Structured native decode failures reach Python as `BacnetDecodingError`. A validated RP/RPM value that fails projection can retain its exact selector and raw value. RPM rows preserve independent local projection and remote property errors. Single and Multiple COV rows always retain received value octets in `raw`. Proven whole-payload truncation in an RP, RPM, ReadRange or decoded AtomicReadFile ACK reports the first absent byte and required/available sizes. Other failures can still be base `BacnetError`. |
| `125d4780a`, `3f9f14175` | The Rust B/IP runtime retains attachment owners and pending shutdown futures through timeout or cancellation. Once admitted, whole-set reconciliation finishes even when its waiter drops. Failed cleanup fences commands and retains unresolved owners. Successful rollback preserves the prior revision/generation. Stop first waits for lifecycle admission, then applies its shutdown budget. Keep the executor alive and retry stop to finish outstanding joins. The native API remains Rust-only. [Runtime contract](../rust-api.md) |
| `979cf2d69` | Valid big-endian UCS-4 CharacterStrings project to Python text. Exact-wire consumers use `read_context.raw` or COV row `raw`. For example, a received UCS-4 Ω retains its original value bytes, while `.octets` emits the canonical UTF-8 write encoding. Unsupported DBCS/JIS values retain their opaque handling. |
| `f4a152d8c` | Private publication verifies the remote source tag and uses the canonical artifacts staged by the tagged release workflow. Extracted source checks and builds carry the pinned Rust toolchain. The local helper builds and validates candidate artifacts. |

### Integration with upstream dev

The release candidate merges upstream `4ade571740ab4cdbe54417cbf171db3b86c4cf7c` into the ACE 0.18.0 preparation at `a3edf6e20bda1cbd99a82c614e396bf023b6428c`. The following decisions apply to the combined source.

| Boundary | Combined behavior |
| --- | --- |
| Routed replies and transactions | Upstream binds replies to the request’s issued path. The full client and server use the upstream transaction and segmented-request machinery. ACE retains the initial SegmentACK for segmented ComplexACKs, a transfer’s fixed service, timer priority and joined endpoint shutdown. A full incoming-request handoff queue counts dropped arrivals while the worker owns the terminal reply. |
| Endpoint segmentation | Endpoints advertise both directions by default. Explicit no-segmentation disables segmented reads and notifications. Receive and both admit large responses. Operators replacing a previously unconfigured ACE endpoint should check the capability advertised on their link. |
| Property values | `read_context.raw` keeps the bytes of an actual ACK. Selector-only contexts have `raw is None`. Contexts compare by selector and property values compare by canonical value and form. Copy/pickle preserve provenance, including published 0.17.2 whole-value pickles. Forced decode takes lowercase datatype tokens such as `unsigned`, `signed` and `character_string`; pass the exact bytes when interpreting a vendor declaration. |
| Network Port tables | BDT and FDT reads, builders and restored 0.17.2 table pickles use upstream’s typed collection and address dictionaries. Empty tables retain their table tag. Exact received bytes remain available in `read_context.raw`. |
| COV and polling | Strict confirmed-COV leases match the wire station, optional router and property notification identity. A one-station sending mark does not change that identity. Upstream managed watch is an opt-in owner of subscription, renewal and polling traffic. Installing a strict expectation alone sends nothing. |
| Local errors and router configuration | Python exposes upstream typed lifecycle errors. A cut-off write that may have gone out requires the caller to resolve its outcome before retry. Native mixed routers now set isolation on each `RouterPort`, and SC unlimited retries use `ScReconnect(max_retries=None)`. |
| Distribution | The ACE distribution keeps its name, Python 3.10 minimum and private index. Its existing tag workflow verifies the source tag and publishes the same staged wheels and source archive used for release validation. |

## Native qualification by artifact and topology

- **Concurrent routed workload.** At `424391ce5`, a shared endpoint exercised
  RP, RPM, ReadRange and AtomicReadFile on four destination networks with
  interleaved segmented COV, a cancelled RP, a file-segment retransmission
  and opposing directions sharing one invoke ID. The same-network case
  submitted competing calls behind one active router/DNET request. Checks
  covered exact values, sequence zero, file offset/EOF, frame loss, admission,
  a usable follow-up read, and shutdown waking pending work. The functional
  wire tests ran on macOS arm64 with the `f90a6b430` SDK. The separate
  measurement harness reports RP-first/RPM-first strata, raw samples, CPU and
  peak RSS. Its matched Linux arm64 `aa5c45236` and `97015c3d3` wheels were
  built. A quiet comparative Python workload run remains open. [Measurement
  protocol](../shared-endpoint-workload.md)
- **Independent software peer.** BACnet4J 6.1.1 on Java 17 served segmented RP
  and two-selector RPM through both client owners against published 0.17.2
  and the `97015c3d3` candidate. The candidate's shared endpoint also accepted
  a registered segmented COV and rejected the unknown subscription.
  [Java fixture, wheel hashes and segment counts](../../scripts/interop/bacnet4j/README.md)
- **Separate host-network containers.** Linux arm64 `97015c3d3` passed the
  two-attachment, common-port test on distinct loopback IPs in the Podman
  guest host namespace. Distinct mapped UIDs returned the correct I-Am source
  and Device name. Occupied bind refusal, failed replacement, readable neighbor
  and restoration all passed. [Host-network fixture](../../scripts/interop/host_network/README.md)

The following three fixtures used the same Linux arm64 CPython 3.13 development
wheel built from `880c92577320549d14a1165c93ca5282d4063974`, SHA-256
`424fe226d2a9d0ad86f062f2ffd093e8f0e447d17f3b829e9cd7698abdaac9d3`.
The installed native extension matched the wheel. The fixture commits identify
the oracle and receipts for each run.

| Fixture | Observation and limits |
| --- | --- |
| [Normal B/IP traffic](../../scripts/interop/traffic_capture/README.md), `d94e3fb21` | In separate loopback-only namespaces each owner produced the expected 29 captured records, including boundary markers. Idle, local health and expectation operations were quiet over the finite interval. The first directed Who-Is emitted What-Is-Network-Number before Who-Is, the second only Who-Is. RP, RPM, explicit subscription/renew/cancel, segmented COV ACKs and late rejection matched the ledger. Capture reported zero kernel drops. An injected unrelated-port datagram failed the audit. |
| [Foreign registration and routed return](../../scripts/interop/foreign_routed/README.md), `aee6b9ce4` | Both local-number profiles remained receive-ready through registration refusal and lease expiry. A routed RP succeeded during lapse. A forwarded I-Am identified its BBMD and the separate return router. Only the configured forwarder's passive COV was admitted, and the strict confirmed COV ACK went to the router. Registration recovered, and a distinct-TTL replacement rebound the same port. The three scripted peers recorded 33 datagrams for the default unknown number and 31 for configured network 7. Only the unknown profile asked for the number on first routed arrival and after recovery. Post-close observation was finite. Real BBMD remote expiry policy remains a deployment check. |
| [Distinct interface broadcasts](../../scripts/interop/distinct_links/README.md), `5118145ad` | Two named-interface endpoints shared port 47808 on separate internal rootless Linux bridges. Both had two required and two active listeners. Limited broadcasts produced the two opposite-interface warnings with actual indices 2 and 3. Limited broadcast, directed broadcast and unicast each admitted only the intended I-Am/COV. Both endpoints replied from their own IP:47808 and the COV streams closed with zero missed events. Three offline oracle controls rejected altered evidence. This run used two disjoint virtual subnets. |

Per-worktree receipts are archived under `dist/verification/<fixture-commit>/`.
The normal traffic fixture retains complete pcaps. The FDR and distinct-link
fixtures retain scripted-peer exchanges and SDK observations.

## Verification of the 0.18.0 implementation

| Scope | Completed evidence |
| --- | --- |
| `97015c3d3` release build only | Five-platform wheel build and installation smokes passed. [Build-only run](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/45707435170) |
| `844a5dc82` standalone request path | Hosted instruction-count gate passed before the Rust runtime and UCS-4 changes. [Performance run](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/45713071918) |
| `979cf2d69` native implementation | Linux CI passed. Local affected-crate checks passed 9,492 tests and 97 doctests, with Clippy and allocation/no-allocation checks. The macOS arm64 CPython 3.13 dev wheel passed 1,206 installed Python tests. Its SHA-256 is `a7291998260fc723b6032501f53d0ea64e0a3c95c038731101d2b0b916f1e5f5`. [Linux CI](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/45716404696) |
| `880c92577` documentation | Python and Rust reference freshness and the production site passed after regenerating the Python reference. Local validation included 485 browser tests. [Docs validation](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/45717646354) |
| `3df09dc54` Windows fixture correction | The Rust runtime suite passed 31 tests locally. Hosted Windows and macOS passed after the exclusive-bind regression recognized Windows error 10013. [Native tests](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/45719976945) |

At `a3989363c`, the completed implementation and handoff passed CI, Native,
Docs with publication disabled, and the standalone instruction-count comparison
against `v0.17.2`. The five-platform release rehearsal also passed.
These jobs used the source revision before the 0.18.0 version bump and the upstream integration. The combined release candidate requires fresh exact-source qualification. The release
commit and its tagged distribution have their own gates and artifact checks.
[CI](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/45725869170) ·
[Native](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/45725872673) ·
[Docs](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/45725876265) ·
[Perf counts](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/45725880427) ·
[Release rehearsal](https://github.com/ACE-IoT-Solutions/rusty-bacnet/actions/runs/45725993168)

## Remaining acceptance work

| Owner | Work and evidence needed |
| --- | --- |
| Edge packaging and configuration | Admit SDK 0.18.0 explicitly, pin the tested distribution and lock container installs on each deployment architecture. Set stable Device/gateway/Network Port identity, bind defaults, forwarding and foreign registration policy. |
| Edge attachment and runtime integration | Connect the Python adapter to the production runtime/factory. Fence and drain work by attachment generation, connect receive and registration health to heartbeat/HTTP, and exercise replacement, failure, rollback and canary recovery. Native Rust lifecycle tests cover the Rust runtime's own owners and joins. |
| Edge polling, discovery and metadata | Retain DriverReadLoop, governor budgets, scheduler priority, adaptive RPM and durable capability learning. Preserve explicit routes, bounded identity enrichment and complete Object_List scans before tombstoning. |
| Edge values and errors | Preserve legacy enum labels, null/empty distinctions, raw and nested vendor values, references, bit names and date/time wildcards. Apply per-property decode outcomes and compare canonical JSON and hashes for schedules, calendars, notifications and references. |
| Edge COV and writes | Install and retain strict leases before subscribe. Own renewal limits, passive/active deduplication, cache and event delivery, cancellation and cleanup across replacement. Preserve write authorization, priority, null relinquish and indeterminate timeout/cancellation outcomes. |
| Edge persistence | Exercise trend MAX → 0 → 1 with checkpoints committed after PubAck. Reproduce four-byte big-endian length framing per record-file record and file-version deduplication. |
| Field and operational performance | Run the deployment's locked Linux x86_64 and aarch64 images on physical interfaces with a vendor controller. Verify source, broadcast isolation, registration and return paths against the site's topology. Compare integrated traffic with the legacy trace and budgets, including sustained overload, cancellation, shared Python polling/COV latency, CPU and RSS on a quiet system. |
