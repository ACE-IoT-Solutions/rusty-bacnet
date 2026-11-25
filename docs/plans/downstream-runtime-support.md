# Downstream BACnet runtime support

The ACE private Python index ships `ace-rusty-bacnet==0.17.2` from commit
`aa5c452364663155b77429729b2dd84f46f74b8f`. Its 0.17.1 base was
`753eec9ff`. The edge migration branch
`feature/rusty-bacnet-migration` records its requirements in
`docs/reviews/bacnet-rusty-full-parity-2026-11-24.md` at local commit
`b718d9dc`. The reference Python backend at `39256f1` includes local receive
readiness (`cf6f40a`) and Linux `SO_REUSEADDR` behavior (`39256f1`).

## Published 0.17.2

CI, Native and the build-only release workflow passed on the published source
commit. The release workflow built and smoke-tested artifacts on five platforms.
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

## Native behavior available to the adapter

| Downstream use | Native contract | Local evidence |
| --- | --- | --- |
| Segmented reads | The client acknowledges segment zero before waiting for the negotiated window. The standalone client and shared endpoint client use this receiver. | With `7b59ac33d`, a bacpypes3 peer limited to 480 service bytes returned identical values through both owners. RP used 13 segments and 7 SegmentACKs. RPM used 26 and 14. The largest response APDU was 485 bytes. |
| Trend and file pages | The same receiver delivers a reassembled service payload to ReadRange and AtomicReadFile. The lenient ReadRange API preserves a reported first sequence number of zero and reports the associated validation violation. | The strict peer fixture waits for ACK 0, 2 and 3. It checks a decoded trend record, raw item data, result flags and sequence zero, and a decoded file window beginning at offset 3072 with 1800 octets and EOF. |
| Local B/IP readiness | A running endpoint exposes receive-loop health, required and active subnet listeners, and multicast socket state via `status()["transport_health"]`. Reading it sends no packets. | At `57a523f73`, local tests exercise listener loss while unicast continues, recoverable invalid input, and stop/restart. Foreign-device registration has its own status API. |
| Confirmed COV | A positive `confirmed_cov_subscription_limit` starts an empty bounded registry. Retain an expectation lease for each confirmed single-object subscription. Matching notifications receive a SimpleACK. Unknown subscriptions receive an Error and produce no event. | At `96b8436e2` and `a3f7eb01e`, Rust and Python tests cover admission, capacity, source qualification, learned or configured local-network aliases, and lease cleanup across seats. |

The initial segmented-read wheel used CPython 3.13 on macOS arm64. Its SHA-256
was `b63d39958898333569747555398154614749ac319f3569569c85e9fb6de659ff`.
The local build record is `dist/segmentation-7b59ac33d-verification.json`.
Twenty downstream contract and parity tests passed in an isolated environment.
The [Python API guide](../python-api.md) documents the endpoint status and
subscription lifecycle.

Strict COV admission currently covers single-object notifications. The strict
client returns unknown-subscription for confirmed multiple-object notifications.
Both confirmed COV forms can be reassembled before admission. An identified
endpoint enables this with Device receive or both. Its default Device still
declares no segmentation. A completed nonzero final segment cannot repeat an
event, while a retransmission of the whole request can.
Unconfirmed events remain available to passive consumers. The adapter owns
SubscribeCOV, renewals, cancellation, process identifier allocation and event
scheduling. Reusing an identifier immediately can admit a late notification
whose wire identity matches the new subscription.

## Candidate qualification

Commit `b0f1bed41` added bounded reassembly of incoming segmented confirmed
COV. At `bcc0422c2`, a full worker queue drops excess segments and requester
Aborts, and endpoint shutdown waits for the worker. Paused-clock tests
exercise a retransmitted final segment with the original still queued, and
a retained client with an active transfer during node shutdown.

On macOS arm64, 4,520 all-feature tests and 45 doctests passed across the
client, endpoint, endpoint-core and server crates. One release timing
benchmark was intentionally skipped. The clean CPython 3.13 release wheel
and probe record are under `dist/bcc0422c2/`. The wheel's SHA-256 is
`14ce13bc0fe1dc62777ef3a41357bbf61239e82efa42ba1774b85ea0c6413f97`.

With this wheel, the original bacpypes3 read probe again returned equal RP
and RPM values through the shared endpoint and standalone client. The RP
response used 13 segments and 7 acknowledgments, and RPM used 26 and 14.
The largest response APDU was 485 bytes. In the opposite direction, the
bacpypes3 peer sent each confirmed COV in 13 segments with a window of two.
Both owners acknowledged sequences 0, 2, 4, 6, 8, 10 and 12, then returned
an unknown-subscription Error or the admitted subscription's SimpleACK. Only
the admitted notification produced an event, including its 6,000-character
description. The peer learned the endpoint's capability from its I-Am.
The standalone probe supplied the peer's cache entry.

On Linux arm64 at the same commit, 4,524 all-feature tests and 11 targeted
transport health tests passed. Both owners passed the segmented COV probe
using a CPython 3.13 debug wheel. A rootless Podman test ran endpoints as
UIDs 10001 and 10002 on separate loopback addresses sharing port 47808.
Both received broadcasts and answered with their own source address and
Device identity. A third UID could not bind an occupied address. After a
failed replacement, the first binding was restored and the second stayed
readable. The logs under `target/downstream-linux/results/` include
`native-bcc0422c2.log`, `health-bcc0422c2.log`, both COV traces and
`cross-uid-bcc0422c2.json`. The tests used an isolated network namespace.

### Standalone performance

The instruction-count harness compared `bcc0422c2` with two byte-identical
baseline builds of `dc3e77de2`. It collected 216 samples using the pinned
image `93ce15bf` through a temporary Docker CLI shim for Podman. Each build
used a separate host-backed volume and two jobs. Measurements ran without
competing builds or tests.

All six standalone client workloads passed. In current-thread and
multithread Tokio runs respectively, the changes in gated instructions per
request were −1.9 and −2.3 for ReadProperty, +0.2 and +2.6 for
ReadPropertyMultiple, and +1.5 and +5.7 for WriteProperty. Each change was
within its measured noise floor, and allocation call counts matched across
builds. The current-thread ReadProperty run also recorded 54 more system
allocator instructions per request, a report-only measure. The complete
results are in `target/downstream-linux/perf-bcc0422c2/summary.md` and
`summary.json`.

## Mixed shared-endpoint workload

Two loopback tests exercise one B/IP socket, an explicitly named router and
strict confirmed COV admission. On four destination networks the peer
receives one RP, RPM, ReadRange and AtomicReadFile before answering any of
them. It sends a late answer to the cancelled RP. The file response and an
incoming segmented COV use the same station and invoke ID in opposite
directions. The peer waits for each initial SegmentACK and withholds file
segment one to force a NAK and retransmission. The client returns all 1,024
file octets at offset 40 with EOF, both RPM rows including the indexed error,
and the original ReadRange record, flags, count and first sequence number zero.

The second test uses one destination network. The routed path admits one
outbound request per router and destination network. While the file holds
that path, the client submits three more reads, receives a segmented COV,
and cancels the RP. The peer observes the file followed by one RPM and one
ReadRange in either order. A subsequent confirmed COV reply bounds the trace.

The tests check the endpoint's UDP source and the destination station on replies,
the admitted COV's provenance and value, and an unknown-subscription Error
for the same registration fields from a different station. A capacity-one
frame subscription reports loss while the COV iterator delivers its next
distinct value. A follow-up read succeeds with zero active leases and a ready
transport. Closing the endpoint wakes an unfinished read and pending COV and
frame subscriptions. Three native tests cover cancelled path waiters, a full
notification worker queue and shutdown while a client handle survives.

Both cases in `test_endpoint_mixed_workload.py` ran on macOS arm64 loopback
with CPython 3.13 and a local wheel built from `f90a6b430`. They cover the exchanges
observed by the test peer. Deployment traffic captures and shared-client
performance qualification remain open. The [shared-endpoint measurement
harness](../shared-endpoint-workload.md) now records routed RP/RPM and
segmented COV latency, endpoint process CPU and peak RSS against a scripted
peer. Matched repeated runs yield local measurements for this specific workload.

## Unreleased Python read diagnostics and property observations

Direct and routed wire tests exercise whole service-payload truncation in
ReadProperty, ReadPropertyMultiple, ReadRange and both decoded AtomicReadFile
access methods on the standalone and shared client. A complete first RPM value
followed by a truncated error row fails the full read. The error reports the
first absent byte and required and available sizes. Published 0.17.2 predates
these diagnostics.

Single and Multiple COV rows preserve the received tagged value octets in
`raw`. For both client owners, the wire tests distinguish a non-minimal
unsigned encoding from its canonical write encoding, a BACnet Null, an
empty octet string and an opaque string. An accepted RPM ACK can also carry
a legacy value that the Python binding cannot project. The affected row
keeps its bytes and reports its local `decode_error`. Neighboring values and
remote property errors retain their own results. Edge projection and
persistence checks remain in the acceptance table below.

## Remaining acceptance work

| Owner | Required evidence |
| --- | --- |
| Edge value and inventory projection | Compare legacy enum labels, raw values, null and empty values and vendor declarations. Preserve Object_List completeness before tombstoning. Compare canonical state JSON and hashes for schedule, calendar, notification and reference projections. |
| Edge trend and file consumers | Exercise a trend cursor through MAX → 0 → 1, committing its checkpoint only after PubAck. Reproduce the existing four-byte big-endian length framing for each record-file record. |
| Edge COV lifecycle | Register and retain leases before subscribing. Exercise renewal, cancellation, late notifications and delivery across attachment replacement. |
| Edge attachment lifecycle | Drain work by attachment generation during replacement. Exercise readiness in heartbeat and HTTP, failed replacement and rollback. Set device identity and router forwarding policy for multi-attachment deployments. |
| Linux deployment | Qualify pinned x86_64 and aarch64 release wheels in the deployed host-network layout, including public interface broadcast isolation, source selection, bind failures and rebind. |
| Wire and performance qualification | Capture idle, discovery and subscription traffic under the downstream workload to check the no-hidden-traffic requirement. Measure shared endpoint and Python workloads with concurrent COV traffic, including latency and resident memory. Exercise physical networks and an independent vendor peer in addition to bacpypes3. |
