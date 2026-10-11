# Embedded router admission baseline

This is a host microbenchmark baseline for workplan C3, not a firmware
throughput, UART timing, stack high-water, or interoperability result. There
is no optimized candidate in this change. Two runs of the same executable
measure repeatability; their difference is not a performance improvement.

## Workloads and guardrails

The harness is `benchmarks/benches/embedded_router.rs`, with static memory
accounting in `benchmarks/benches/embedded_router/memory.rs`. It uses Criterion
0.8.2 with 30 samples per workload, 500 ms warmup, and a requested one-second
measurement interval per workload (27 workloads). Criterion chooses the
iteration count and reports bootstrap 95% confidence intervals. Setup and
correctness assertions occur outside the measured loop; arguments and results
cross `black_box` boundaries. No random workload seed is needed.
Workloads run in a fixed order with warmed, repeatedly accessed state; this
does not estimate cold-cache latency or a mixed arrival distribution.

| Path | Inputs and timed operation |
|---|---|
| Direct forwarding | Both route partitions full at 2, 8, or 32 entries each; lookup of last direct route; parse/rewrite plus one ownership copy; incoming NPDU 64 or 497 bytes, final length +4 |
| Learned forwarding | Same full tables; lookup of last learned route after direct lookup; parse/rewrite plus one ownership copy; incoming NPDU 64 or 497 bytes, final length +9 |
| Broadcast fanout | 64-byte global broadcast, 2/4/8 successful egress copies with cache capacity 8/32/128; supplied clock advances by retention (10 units), measuring expiry, insertion, rewrite, and fanout every call |
| Duplicate lookup | Full cache capacity 8/32/128; last-entry hit or distinct-key miss with deterministic replacement; constant clock, no expiry; includes production counters |
| Saturated queue | Concrete `MstpEgress` and heapless `ProfileMasterCore`; unicast capacities 1/4/8; 501-byte standard or 1497-byte explicitly enabled extended request; queue is full before measurement and clock stays fixed |

The forwarding sink owns accepted bytes in fixed storage and consumes the
previous packet synchronously. It models a copy at the admission boundary;
it does not model MS/TP service time. Both forwarding cases use an extended
port ceiling so the 506-byte learned output is admitted. Broadcast uses
standard port ceilings. Broadcast timing exercises one expiring identity,
not a full cache; the separate duplicate cases cover full occupancy.

The saturated setup asserts that all expected data slots admit, the next
request returns `Full`, and one network-control request still admits into
the separate two-slot reserve. It verifies unchanged data depth after the
run. The forwarding and fanout setup asserts actual success and final
length/fanout count. The smoke command runs all these assertions without
collecting timings. The router, adapters, and MS/TP all-feature test suites
also passed before measurement preparation.

## Environment and reproduction

Measured production revision: `ec5046f3`, with the uncommitted benchmark
harness and three new benchmark dependency entries. `environment.txt`
records the full revision and dirty state; `source-sha256.txt` identifies
every relevant source/manifest and `executable-sha256.txt` identifies the
single executable used for both runs. All captured source hashes still
matched after both timing passes (`source-verification.txt`).

Host: Apple M4 Pro, 48 GiB RAM, arm64 macOS Darwin 25.6.0. Toolchain:
Homebrew rustc 1.98.1 (`48a229cea`, LLVM 22.1.8), cargo 1.98.1
(`797e8a9bc`). Default Cargo optimized bench profile; no custom target CPU,
allocator, CPU affinity, or frequency control. Raw environment capture and
source hashes accompany the timing logs. Power, background virtual-machine
activity, and scheduling are uncontrolled; run-to-run variance must be
retained. Results apply only to this host and build.

From repository root:

```sh
cargo check -p bacnet-benchmarks --benches --locked
cargo bench -p bacnet-benchmarks --bench embedded_router --locked -- --test
cargo bench -p bacnet-benchmarks --bench embedded_router --locked -- --save-baseline router_run_a
cargo bench -p bacnet-benchmarks --bench embedded_router --locked -- --save-baseline router_run_b
ROUTER_BENCH_MEMORY_ONLY=1 cargo bench -p bacnet-benchmarks --bench embedded_router --locked
cargo test -p bacnet-router-core -p bacnet-router-adapters -p bacnet-mstp-core --all-features --locked
cargo clippy -p bacnet-benchmarks --bench embedded_router --no-deps --locked -- -D warnings -A missing-docs -A clippy::print-stderr -A clippy::print-stdout
```

The scoped clippy command permits three existing lint categories in the
benchmark package's hosted stress library. Without those allowances, the
package fails on 81 existing missing-docs/printing violations; the strict
failure log is retained. No production lint configuration changed.

For the recorded passes, the release smoke command built the executable
first; measurement then invoked that same executable directly:

```sh
target/release/deps/embedded_router-2cdc5afba697801a --bench --save-baseline router_run_a
target/release/deps/embedded_router-2cdc5afba697801a --bench --save-baseline router_run_b
```

The filename hash is build-specific; the Cargo commands above are the
portable reproduction path. Each recorded pass took 50 seconds: run A at
05:54:21–05:55:11 UTC and run B at 05:55:19–05:56:09 UTC on 2026-10-11.
`run-a.txt` and `run-b.txt` contain the full console observations.
`criterion-raw.tar.gz` contains the 54 saved-baseline directories (27 per
run), each with `benchmark.json`, `estimates.json`, `sample.json`, and
`tukey.json`, retaining all 30 samples and iteration counts. Paths in the
archive are relative to the repository root. Compressed check, smoke, and
lint logs retain build verification output; `final-smoke.log.gz` is the
smoke run for the measured revision.

The complete mean/95% confidence-interval comparison is in [results.md](results.md).
Across identical-binary repetitions, mean differences ranged from −3.2%
to +5.8%. In particular the 128-entry last-hit duplicate workload shifted
from 99.06 ns to 104.83 ns, with non-overlapping within-run intervals.
Within-run bootstrap uncertainty therefore understates the total host
repeatability uncertainty. These deltas do not establish an optimization
or regression. A future A/B effect near this range requires interleaved
quiet-host runs before interpretation.

## Memory accounting

All sizes are bytes from `size_of` on the 64-bit host, not an embedded ABI
promise. `R` is the capacity of **each** direct/learned partition and pending
discovery; `C` is cache capacity; `P` is diagnostic/fanout capacity; `Q` is
each unicast/broadcast data queue capacity, with two extra control slots.
These are independent stress knobs, not recommended deployment profiles.

The report prints route storage, router including route storage/counters,
pending discovery, duplicate cache, broadcast policy including that cache,
operational diagnostics, fanout result scratch, control action, detached
network/route scratch for 2R entries, and Initialize ACK wire scratch for R
direct networks. It also prints queue storage separately from the core
which contains it. A diagnostic snapshot borrows caller storage; no
additional heap allocation is implied.

Illustrative composition totals add router + discovery + broadcast policy +
diagnostics + the listed action/control scratch + **one** MS/TP core + one
NPDU rewrite buffer (501/1497) + one receive buffer (511/1516). Contained
routes, cache, and queues are not double-counted. The total is a sum of
components, not a measured struct layout or stack maximum. It excludes
alignment between separately allocated components, application pending
packet storage, UDP/lwIP queues, tasks/stacks, UART/DMA, allocator overhead,
OS state, and additional MS/TP cores for additional MS/TP ports.

The exact profile lines are retained in each timing log. Future embedded
consumers should rerun layout accounting for their ABI and add their own
bounded packet buffers and platform allocations before choosing a budget.

| R / C / P / Q | Standard core | Extended core | Core delta | Standard composition | Extended composition |
|---|---:|---:|---:|---:|---:|
| 2 / 8 / 2 / 1 | 12,824 | 35,888 | +23,064 | 15,181 | 40,246 |
| 8 / 32 / 4 / 4 | 16,088 | 45,152 | +29,064 | 21,197 | 52,262 |
| 32 / 128 / 8 / 8 | 20,440 | 57,504 | +37,064 | 36,045 | 75,110 |

## Interpretation and next measurements

The useful outputs are a repeatable harness, raw host observations, and
explicit fixed-capacity layout costs. No optimization or latency guarantee
is proposed. Compare a future candidate against this baseline only with
the same workload definitions, toolchain, target, features, and host; rerun
baseline and candidate interleaved under quiet conditions. The next device
measurement should use the selected firmware capacities and include queue
service, scheduler latency, UDP delivery, UART timing, and stack high-water
under sustained bidirectional traffic and control bursts.
