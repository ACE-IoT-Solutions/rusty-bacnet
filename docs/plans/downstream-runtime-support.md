# Downstream BACnet runtime support

The ACE 0.17.1 candidate starts at `753eec9ff`. The edge migration branch
`feature/rusty-bacnet-migration` records its requirements in
`docs/reviews/bacnet-rusty-full-parity-2026-11-24.md` at local commit
`b718d9dc`. The reference Python backend at `39256f1` includes local receive
readiness (`cf6f40a`) and Linux `SO_REUSEADDR` behavior (`39256f1`).

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
Incoming segmented confirmed COV requests remain a separate receive limitation.
Unconfirmed events remain available to passive consumers. The adapter owns
SubscribeCOV, renewals, cancellation, process identifier allocation and event
scheduling. Reusing an identifier immediately can admit a late notification
whose wire identity matches the new subscription.

## Remaining acceptance work

| Owner | Required evidence |
| --- | --- |
| Edge value and inventory projection | Compare legacy enum labels, raw values, null and empty values and vendor declarations. Preserve Object_List completeness before tombstoning. Compare canonical state JSON and hashes for schedule, calendar, notification and reference projections. |
| Edge trend and file consumers | Exercise a trend cursor through MAX → 0 → 1, committing its checkpoint only after PubAck. Reproduce the existing four-byte big-endian length framing for each record-file record. |
| Edge attachment lifecycle | Drain work by attachment generation during replacement. Exercise readiness in heartbeat and HTTP, failed replacement and rollback. Set device identity and router forwarding policy for multi-attachment deployments. |
| Linux deployment | Build and install pinned x86_64 and aarch64 wheels. Exercise rootless multi-UID listeners on port 47808, per-interface broadcast isolation, source address selection, bind failures and rebind. |
| Wire and performance qualification | Capture idle, discovery and subscription traffic under the downstream workload to check the no-hidden-traffic requirement. Run representative performance and physical network qualification. |

Local macOS loopback results cover the transactions above. Linux, rootless
container, physical network and performance qualification remain open.
