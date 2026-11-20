# ACE integration on upstream dev

This integration starts at upstream `bbc8b5c803c633aa14a2bacbbdc954615c380a38` and carries maintained ACE behavior from `7c1e6c24aa89590d16c07aa77ef6d4b0f789ae15`. The source workspace retains the ACE version 0.15.3. The rebase creates no release or tag. The workplans under `docs/plans/` record earlier design decisions.

## Protocol and API coverage

Upstream supplies the full client and server roles on one endpoint socket, discovery scopes and per-I-Am arrival streams, routing-table and BDT/FDT reads, and routed property, range and atomic file reads. Callers can name a router or let the client learn the next hop. The ACE integration carries routed WritePropertyMultiple and exposes both NPDU address pairs on I-Am arrivals. Previously persisted arrivals still load with absent destination fields.

The Python endpoint roles use the upstream classes, direct address strings and `RoutedTarget(network, address, router=next_hop)`. Frame consumers use the upstream time and loss fields. See the [binding migration](../changelog.d/2612-ace-shared-endpoint-migration.md), [arrival migration](../changelog.d/2612-iam-arrival-destination-migration.md) and [Python API](python-api.md).

| ACE behavior | Preservation |
| --- | --- |
| Native B/IP runtime | The private `bacnet-runtime` crate supports managed attachments, explicit routed operations, ordered outcomes, cancellation, rollback and retry. Its calls use the upstream `Station` type. |
| Mixed B/IP and SC router | Per-port health, counters, isolated routing audiences and per-port recovery announcements remain available. |
| Frame capture | SC endpoint and mixed-router records carry decrypted frame bytes, capture-time hub URL, port identity and Unix nanoseconds where available. Closing a tap synchronizes its terminal loss count. |
| Packaging | The private Python distribution is `ace-rusty-bacnet` and its import is `rusty_bacnet`. Repository guards separate ACE and upstream release destinations. The wheel and repaired source archive include third-party notices and the source archive supports locked offline dependency resolution. |
| Dependency maintenance | The workspace carries the patched heapless and rustix versions. Cargo Deny 0.21 checks the expiring, package-scoped chrono exception. The audit treats unsound advisories as errors. |

The uncommitted TLS-attempt diagnostics prototype is preserved outside this integration. Carrying its accepted and rejected peer identities across reconnects requires a native dial-attempt observation hook.

## Local verification

On the first integration candidate, default-feature workspace nextest passed 15,865 of 15,866 tests. The remaining assertion expected the old I-Am pickle length. The corrected reconstruction and router isolation tests both passed on the repair. Of 1,159 Python tests, one source inventory still referenced the former endpoint module layout. All six future-contract tests passed after its file list included the extracted observation module. The strengthened file cancellation and malformed-response tests also passed.

The initial package passed 193 release-tool tests, a locked offline source-archive Cargo check, Cargo Audit and Cargo Deny. The macOS feature suite, doctests, final installed wheel and rebuilt source archive are separate final-head checks. For the native features and commands, see [local macOS checks](../scripts/ci/local-macos.sh). Release artifacts can be checked locally with `scripts/release/repair_sdist.py`, `cargo build --locked --offline -p rusty-bacnet` in the extracted source, and `python -I scripts/release/wheel_smoke.py --version 0.15.3` in the wheel environment.
