# Address-copy experiment for #1978

**The production optimization was rejected under the fixed acceptance protocol.**
Replacing eight reached ownership copies reduced Linux ReadProperty instruction
counts with unchanged allocation traffic, but the native latency observations did
not establish the required overall nonregression. The delivery change contains
only this evidence, its index entry and changelog. Production and product-test
files were restored byte-for-byte to the baseline. This is Part of #1978; its
other costs remain open.

The native result does not establish a causal slowdown. WriteProperty's aggregate
median decreased but its uncertainty exceeded the declared bound;
ReadPropertyMultiple's aggregate median increased and its interval was wider.
No observation was removed, no matrix was repeated, and no threshold was changed.

## Source and measured behavior

| Variant | Immutable commit | Meaning |
| --- | --- | --- |
| A | `2d82e9a949bb57702f4672705a46b19345bb3d59` | Unchanged baseline |
| B | `b1f6845eca586bc03b98c4445e82b54fd49fdc98` | Rejected measured candidate; tree `54d9c7c240825d2fc98b2bdeea657bf08c2ac34e` |

`candidate.patch.gz` contains the exact patch bytes in deterministic gzip form
(mtime zero). Decompress it before applying it to reconstruct all eight B files
exactly from A, including its
three added identity tests and historical optimization changelog. Apply it only
to an isolated experimental checkout. Its seven Rust files are not present as
changes in the final delivery diff. The original candidate remains in branch
history; the later evidence commit explicitly restores A's production bytes.
The raw patch hash is unchanged. Compression preserves blank context markers
without introducing trailing whitespace into the tracked text artifact.

The public `MacAddr = SmallVec<[u8; 6]>` alias, dependency versions and features
were unchanged. B used the existing safe `from_slice` constructor at the reached
client request/guard, transaction-peer, ComplexACK key and server receive copies,
and manual byte-preserving `Clone` implementations for the two canonical peer
identities. No ownership was borrowed, and no admission/security/lock behavior
was changed. Tests covered lengths 0/1/3/6/7/18/23/64, inline and spilled storage,
network/variant/length/trailing-byte distinctions, map lookup and mutation
independence, including the 23-byte routed transaction key for an 18-byte address.

The separate exploratory A current-thread profile attributed 7,018,976 self
instructions to the generic `SmallVec<[u8; 6]>` byte-`Extend` frame over 10,202
requests: eight calls and 688 frame instructions per request. Source/caller
attribution reached transaction-peer construction twice, request registration and
its guard once each, server receive/admission once each, coordinator admission
once, and the client ComplexACK key once. The selected final B profile no longer
contains that frame. Other SmallVec uses are outside this attribution. The net
end-to-end instruction saving below is not an additive claim that every frame
instruction disappeared without replacement or other code-generation effects.

## Frozen protocol and environment

Both matrices used `A B B A` repeated three times, six invocations per variant.
Pairing uses equal per-variant ordinals. Owned builds and benchmark suites ran serially, with root reporting no
competing work. Pre/post process snapshots support that account; they are not
continuous proof excluding every unrecorded source of host interference.
All 60 planned invocations exited zero, but execution success
is distinct from the failed overall performance acceptance.

- Callgrind: ReadProperty on current-thread (`ct`) and two-worker multi-thread
  (`mt`) Tokio runtimes, 200 warmups, separate N=1000/10000 invocations; each
  slope is `(Ir10000 - Ir1000) / 9000`. All six B slopes must be lower than their
  A pair, and the difference of median slopes must exceed A's max-minus-min
  repeated slope drift on each runtime. Matched median allocation-call and
  requested-byte traffic at N=10000 must not increase; allocation slopes are
  also retained. This definition was encoded before timing.
- Native: the actual unmodified `bip_latency` Criterion binary, RP/WP/10-object
  RPM, `TOKIO_WORKER_THREADS=2`, sample size 100, warmup 3 seconds and measurement
  5 seconds. For each workload B's median of invocation medians must not exceed
  A's, and the paired bootstrap 95% interval's upper slowdown bound must be at
  most 2%. That bound does not permit a higher observed median.
- Bootstrap: enumerate all `6^6 = 46,656` ordered samples of six paired ordinals
  with replacement. Each statistic is `median(B) / median(A) - 1`; endpoints
  are linearly interpolated 2.5th and 97.5th percentiles. It is not a bootstrap
  of pooled Criterion samples. Every invocation's original estimate and sample
  bytes remains available, as do all paired differences and ranges. These are
  Criterion samples of repeated operations, not 3,600 individual request latencies.

The host was an Apple M5, 10 CPUs, 16 GiB RAM, macOS 27.2 (26B118),
`aarch64-apple-darwin`, Rust/Cargo 1.99.0, rustc `b940084d7`, LLVM 23.1.1.
Linux Callgrind ran in arm64 Docker 29.8.2 with four assigned CPUs and
5,155,708,928 bytes of VM memory, Rust 1.99.0/LLVM 23.1.1 and Valgrind 3.22.0.
The task derivative image ID and actual executable paths/hashes are in
`inputs.json`. Builds used separate targets, `CARGO_INCREMENTAL=0`, the identical
normalized external harness lock and manifests, and no feature override.
Native compiled features/profiles matched for all 226 units. Linux's common
release profile adds line-table debug information; per-unit Linux feature
metadata was not separately captured, and the full inherited native build
environment was not archived. `build-inspection.json` states the recorded evidence;
it is not a complete verbose compiler-invocation/environment graph.

**Power observation remains unresolved.** The pre-matrix snapshot reported
`Battery Power`, 100%. Every native invocation's header reported `AC Power`,
while its battery detail simultaneously said `discharging`, falling from 100%
to 97%. The post-matrix snapshot again reported `Battery Power`, 97%. Original
strings are preserved in `runs.json` and `provenance.json`. They are not
normalized into steady AC, asserted to cause the latency differences, or used
to exclude any run. No affinity/frequency/system-power setting was changed.

The Linux command checks its executable hash before each invocation. The native
runner checks both binaries and full source inventories before and after the
matrix, not before every invocation. The original independent archive review
verified recorded guards/hashes, not archived executables: none were in that
initial archive. After timing, four retained executables were copied and rehashed
as supplemental local evidence; those later copies are not part of that original
archive review or the distributed Git payload.

The common harness checks the actual AI1 value (21.0), response identity and
property/index before and after the counted loop. Its WP control verifies AO1
readback (72.5). These checks passed for A/B on ct/mt before final measurements.
The timed native workloads are the existing RP/WP/RPM benchmarks, unchanged.
Whole-program instruction slopes remove fixed setup/oracle work; allocation
counters cover the N-request loop across all threads, including background work.
Requested allocation traffic is not retained/peak memory, and the hosted round
trip is not allocation-free.

## Results

Instruction values are median [minimum–maximum] instructions/request:

| Runtime | A | B | Median saving | Repeated A drift | Six paired savings range | Result |
| --- | --- | --- | --- | --- | --- | --- |
| ct | 40406.768 [40403.548–40415.257] | 39886.577 [39882.238–39891.203] | 520.192 | 11.710 | 514.809–526.886 | Pass |
| mt | 41490.685 [41484.690–41494.730] | 40970.771 [40967.919–40976.715] | 519.914 | 10.040 | 514.848–522.059 | Pass |

Matched median allocation calls/request and requested bytes/request were exactly
unchanged: ct **22.0002 / 2228.0208**, mt **22.0003 / 2228.0312**, on both A/B.
All raw N=1000 and N=10000 instruction/allocator totals, allocation slopes and
ranges are distributed; fractional counts reflect division and background work.

Native values are median [minimum–maximum] of six invocation medians, in ns.
Slowdown is the ratio of aggregate medians minus one; negative means lower B.

| Workload | A | B | Aggregate slowdown | Paired bootstrap 95% interval | Observed median gate | Upper-bound gate |
| --- | --- | --- | --- | --- | --- | --- |
| RP | 26487.684 [26029.578–27527.951] | 26087.477 [25910.543–27287.388] | -1.5109% | [-2.5175%, +0.0995%] | Pass | Pass |
| WP | 29119.155 [27451.532–31099.326] | 27473.507 [27440.454–32033.241] | -5.6514% | [-9.6609%, +3.0030%] | Pass | **Fail** |
| 10-object RPM | 32317.326 [31396.139–34858.549] | 32554.345 [31202.619–35148.168] | +0.7334% | [-5.7972%, +8.8973%] | **Fail** | **Fail** |

Absolute paired B-minus-A median differences ranged from -669.324 to +176.557 ns
for RP, -2934.794 to +933.915 ns for WP, and -2053.431 to +3352.872 ns for RPM.
The instruction/allocation gates pass; RP's native gates pass; WP and RPM fail
the stated native qualification. **Overall: not accepted.** There is no claimed
platform-wide speedup, physical-network qualification or statistically established
causal RPM regression. A future experiment would require a separately chosen
scope and protocol; none was added to this dataset.

## Correctness and retained failures

Before timing, A passed 4,105 scoped tests and B passed 4,108, each across client,
server, endpoint-core and encoding with all features. Each passed 43 doctests.
B passed strict affected-crate Clippy/private rustdoc, two existing no-allocator
codec exchange tests and the existing envelope-layout budget test. Relevant
transaction cancellation/segmentation/admission and future-size tests are in the
scoped suites. These checks describe the historical A/B sources, not a new Cargo
gate for the final evidence-only delivery head. The codec subset's zero-allocation
contract does not describe the hosted round trip above.

The first Docker build failed on a sandboxed `~/.docker/buildx` activity-file
write. A task-local `BUILDX_CONFIG` allowed the authorized derivative build;
the shared image was unchanged. Initial external-harness resolution was explicitly
unlocked: it added only `rp-harness`, removed 186 unused workspace records and
changed no retained version/checksum. A subsequent `--locked` build succeeded.
The first worktree file write and an early process read were also sandbox-denied;
reviewed authorized retries succeeded before timing. No performance run failed,
was replaced or was discarded. `failures.json`, `image-build-failure.log`,
`lock-normalization.json` and `commands.json` retain classifications and evidence.

## Distributed and local evidence

- `inputs.json`: original full A/B source hashes, lock/harness/profile identities,
  image/compiler/environment records and exact native/Linux executable hashes.
- `runs.json`: all 48 Callgrind totals/counters and all 12 native invocation
  records, exact commands/statuses/timestamps, original stdout/stderr and power.
- `criterion.jsonl`: original bytes and SHA256 hashes of all 36 estimates and
  sample files, containing all 3,600 positive samples. `summary.json` contains
  every invocation median, paired difference/range and acceptance result.
- `candidate.patch.gz`, `patch-reconstruction.json`: the rejected candidate and its
  exact eight-file reconstruction proof. `common/` holds the unchanged harness,
  normalized lock, profile, protocol and derivative Dockerfile.
- `measured/`: byte-identical original runners and analysis. Top-level runners
  only parameterize the capture directory; `capture.py` is a packaging helper.
- `profiles/`: compressed original exploratory A and selected final B ct profiles,
  with raw/compressed hashes in `provenance.json`; `attribution.json` identifies
  the reached frame. Complete original profiles and binaries remain at the local
  evidence path in provenance, approximately 70 MiB of output. They are not an
  uploaded/downloadable artifact. Root preserves that archive before cleanup.
- `sha256.json`: distributed artifact hashes. `reconstruct.py` verifies original
  bytes, source identities, run order/status, per-run counters, sample counts and
  reconstructs the complete frozen result without compiling or timing.

The isolated integrity challenge reseals outer checksums after changing one metric,
source identity or sample, so rejection exercises semantic guards. All three
corruptions were rejected, and the untouched data passed before/after.
`integrity-validation.json` retains these expected failures. The capture helper
also accepted exact A/B source and rejected a wrong revision before restoration.

Run from any directory with Python 3.10+:

```sh
python3 /absolute/path/to/1978-macaddr/reconstruct.py
python3 /absolute/path/to/1978-macaddr/validate_integrity.py
```

## Opt-in reproduction of new measurements

Reconstruction above uses the published observations. The following is an
explicit new experiment, not a favorable retry of this dataset. Use clean isolated
checkouts at A and B, private native targets and fresh Linux volumes. The original
B commit is retained in history; `candidate.patch.gz` provides its exact intervention.
`capture.py` refuses a wrong revision, source hash, unsuccessful Cargo JSON build,
reused variant label or mismatched common harness.

Linux reproduction requires the retained `rb-callgrind:local` base image
`sha256:0f6ec7193b18244427fba499eaf40ff35be670e08b327c5f689b1a6eb9c5f5ee`.
It is a local prerequisite, not a public registry download. The distributed
Dockerfile installs pinned Rust 1.99.0 in a derivative, preserving the base.
An independently constructed image is a new environment and must be identified
as such. Probe actual compiler/Valgrind versions and preserve its image ID.

Set `EVIDENCE` to this directory, `FROZEN` to a new absolute capture directory,
and `A_TREE`/`B_TREE` to the clean immutable checkouts. No path may reuse the
original experiment's targets, volumes or outputs. Build all variants serially,
check every command's status, and retain stdout/stderr. For each label, use its
`TREE`, private `NATIVE_TARGET`, fresh `VOLUME` and selected `IMAGE`:

```sh
# Prepare FROZEN/harness once by copying common/harness with its existing lock.
# Verify the retained base ID before building the task-local derivative.
BUILDX_CONFIG="$FROZEN/buildx" docker build --progress=plain \
  -t "$IMAGE" "$EVIDENCE/common/image"

cd "$TREE"
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$NATIVE_TARGET" \
  cargo +1.99.0 bench --locked -p bacnet-benchmarks --bench bip_latency \
  --no-run --message-format=json > "$FROZEN/$LABEL-build.json" \
  2> "$FROZEN/$LABEL-build.err"
# Create VOLUME only after confirming that name does not already exist.
docker volume create "$VOLUME"
docker run --rm -e CARGO_INCREMENTAL=0 -e CARGO_TARGET_DIR=/target \
  -v "$TREE:/wt:ro" -v "$FROZEN/harness:/h:ro" -v "$VOLUME:/target" \
  -w /h "$IMAGE" cargo build --release --locked
python3 "$EVIDENCE/capture.py" "$LABEL" "$TREE" "$FROZEN/$LABEL-build.json" \
  "$NATIVE_TARGET" "$VOLUME" "$FROZEN" --image "$IMAGE"
```

Before timing, repeat the recorded correctness commands and ct/mt RP/WP harness
oracles, then capture the qualified binaries after all builds finish. Check actual
OS/toolchain/power/process/container state and at least 50 GiB free. Use no other
builds/benchmark suites or system-setting intervention. The fixed serial matrix is:

```sh
python3 "$EVIDENCE/callgrind_matrix.py" "$FROZEN"
python3 "$EVIDENCE/native_matrix.py" "$FROZEN"
python3 "$EVIDENCE/analyze.py" "$FROZEN"
```

The runners refuse existing output/baseline names, stop on execution failure,
retain raw data and check frozen source/binary identity. New build paths may alter
binary layout/hash; capture the actual new executable rather than expecting the
original machine's byte identity. Keep the resulting new data separate, including
any failures or uncertainty. No new measurements are needed to verify this report.
