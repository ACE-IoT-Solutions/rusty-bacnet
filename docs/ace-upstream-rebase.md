# ACE integration on upstream dev

This integration starts at upstream `bbc8b5c803c633aa14a2bacbbdc954615c380a38` and carries maintained ACE behavior from `7c1e6c24aa89590d16c07aa77ef6d4b0f789ae15`. ACE 0.16.0 adopts the upstream endpoint and addressing APIs. The workplans under `docs/plans/` record earlier design decisions.

## Protocol and API coverage

Upstream supplies the full client and server roles on one endpoint socket, discovery scopes and per-I-Am arrival streams, routing-table and BDT/FDT reads, and routed property, range and atomic file reads. Callers can name a router or let the client learn the next hop. The ACE integration carries routed WritePropertyMultiple and exposes both NPDU address pairs on I-Am arrivals. Previously persisted arrivals still load with absent destination fields.

The Python endpoint roles use the upstream classes, direct address strings and `RoutedTarget(network, address, router=next_hop)`. Frame consumers use the upstream time and loss fields. Rust callers constructing a `DecodedNpdu` literal must initialize its new `dest_address` field. See the [Python migration guide](python-api.md#migrating-ace-0153-capture-and-routed-calls).

| ACE behavior | Preservation |
| --- | --- |
| Native B/IP runtime | The private `bacnet-runtime` crate supports managed attachments, explicit routed operations, ordered outcomes, cancellation, rollback and retry. Its calls use the upstream `Station` type. |
| Mixed B/IP and SC router | Per-port health, counters, isolated routing audiences and per-port recovery announcements remain available. |
| Frame capture | SC endpoint and mixed-router records carry decrypted frame bytes, capture-time hub URL, port identity and Unix nanoseconds where available. Closing a tap synchronizes its terminal loss count. |
| Packaging | The private Python distribution is `ace-rusty-bacnet` and its import is `rusty_bacnet`. Repository guards separate ACE and upstream release destinations. The wheel and repaired source archive include third-party notices and the source archive supports locked offline dependency resolution. |
| Dependency maintenance | The workspace carries the patched heapless and rustix versions. Cargo Deny 0.21 checks the expiring, package-scoped chrono exception. The audit treats unsound advisories as errors. |

The uncommitted TLS-attempt diagnostics prototype is preserved outside this integration. Carrying its accepted and rejected peer identities across reconnects requires a native dial-attempt observation hook.

## Local verification

Before the version bump, the macOS feature suite passed 15,727 native tests and the installed wheel passed 1,159 Python tests. Both the wheel built from the checkout and the wheel rebuilt offline from its repaired source archive passed the release smoke test. See the [local macOS checks](../scripts/ci/local-macos.sh) for native feature selection.

For a local packaging check, run `cargo fetch --locked` to populate the Cargo
cache, then `scripts/build-python-release.sh <version>`. The script uses
the release-pinned maturin and the Git commit timestamp. It prunes the
sdist’s workspace lockfile offline, compiles from that archive and
smoke-tests the local wheel. `--wheel-dir DIR` adds supplemental wheels
for metadata checks. `--allow-dirty` permits a local working copy for
validation. Each run writes its files to a fresh directory under
`dist/python-build/<version>/`.

The tagged GitHub release workflow publishes ACE distributions. Its private
index job checks that `origin` advertises `v<version>` at the tested commit
and verifies the downloaded release assets against the staged GitHub
release’s SHA-256 manifest. It builds a smoke wheel from the canonical
archive with the locked, cached dependencies. Immediately before uploading
the original sdist and ten CPython 3.10/3.13 wheels, it rechecks the manifest,
checkout and remote tag. An existing index file with a different hash stops
the upload. Corrections to published artifacts require a new version.

To check an installed ACE 0.16.0 wheel, run `python -I scripts/release/wheel_smoke.py --version 0.16.0`.
