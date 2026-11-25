# Shared-endpoint polling and confirmed COV measurements

Run `scripts/perf/endpoint_workload.py` with the Python interpreter that has
the wheel installed. Each invocation starts a fresh process with one B/IP
endpoint and a separate Python UDP router process on ephemeral IPv4 loopback
ports. The router uses the standard library. Both run on the same Linux or
macOS kernel and use its monotonic clock.

```sh
mkdir -p results
"$ARM_PYTHON" scripts/perf/endpoint_workload.py \
  --wheel "$ARM_WHEEL" --source-sha "$ARM_SHA" \
  --warmup 30 --rounds 500 --seed 1 --output results/sample-01.json
```

The source SHA is the build operator's declaration. The program hashes the
supplied wheel, checks that its native extension member matches the installed
binary byte for byte by SHA-256, and records the runtime SDK version,
interpreter, ABI, OS, architecture and both harness script hashes. Keep the
build record with the wheel. A source SHA cannot be recovered from the native
extension hash alone.

## Exchanges and measurements

One round submits ReadProperty and ReadPropertyMultiple to the same router,
DNET and station. The shared client admits one read at a time on this path.
The peer accepts either admission order and holds the first read while it sends
a segmented ConfirmedCOVNotification using that read's invoke ID. A strict
registration expects the peer, process, device and monitored object. The peer
requires the final SegmentACK and terminal SimpleACK before replying to the
held read. Each of the two read responses is also segmented.

Each service returns its own 1,440-byte Description with a seed and round
identity. RPM returns three ordered selectors for Device 10, including an
unsigned Object_List length at index zero and a Present_Value property error.
Every receiver checks the value, source or selector fields that apply to its
service. Each transfer uses 480-byte payload chunks and a window of two. The
peer waits for ACK 0 and records the subsequent window and final ACKs for each
invoke. The endpoint checks the COV iterator's loss count, active transaction
leases and transport readiness after the final round.

The JSON contains warmup and measured rows. The summary uses completed
measured rounds. The latencies, in nanoseconds, cover submission through
Python value validation for each read, and the peer's first COV send through
Python event validation for COV. Read latency includes time queued on the
shared route. Raw timestamps, value digests, read admission order and exact
peer counts are retained. The p50, p95 and p99 summaries use nearest ranks.
A correctness failure returns a nonzero process exit code and writes the
failed phase, available round data and exception to the output file.

Endpoint process CPU is the change in user and system CPU during each phase,
including Python's validation and sample collection. Peak RSS is the process
lifetime high-water mark, so startup, provenance checks, warmup and retained
samples can contribute. The report also records the high-water mark at phase
start. RSS units are bytes on macOS and KiB on Linux. The peer's process
resources are excluded; the two processes share the host scheduler.

## Comparing builds

Build A and B on the same machine or pinned image with the same architecture,
CPython ABI, Cargo and rustc 1.99.0, maturin 1.15.0, Cargo feature set, locked
dependencies, compiler flags and profile. Record the build commands and
environment. Install each wheel in its own environment. Use the same frozen
harness scripts for every arm. The published 0.17.2 wheel can check baseline
API compatibility. A published release wheel and a local dev-profile wheel
serve different build configurations.

Run one sample at a time on a quiet machine. Use fresh interpreter processes,
a common seed, the same warmup and measured counts, and several alternating
blocks. For example, run A, A as a same-wheel noise control, then A, B, B, A.
Repeat that block with distinct filenames. For two independent builds of the
baseline use A, A′, then A, B, B, A′, and retain each wheel's hash. Use enough
rounds for CPU accounting and tail ranks to stabilize; 500 measured rounds
after warmup is a starting point. Report per-sample results and the observed
spread across repeats. Evaluate successful samples separately from failures.
The harness sets no latency, CPU or memory threshold.

A short artifact correctness check runs the CLI, verifies three exchanges,
checks the peer's response to an invalid SegmentACK, and rejects a wheel
whose native binary differs from the installed extension.

```sh
ENDPOINT_WORKLOAD_TEST_WHEEL="$ARM_WHEEL" "$ARM_PYTHON" \
  -m unittest discover -s scripts/perf -p test_endpoint_workload.py
```

The wire ledger covers traffic received or sent by this scripted peer during
the specified exchanges. Production startup and idle traffic captures,
deployed network routing, independent vendor behavior and Edge processing
require separate qualification.
