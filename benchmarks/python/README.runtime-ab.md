# Clean-wheel runtime A/B evidence harness

This harness compares sequential chatty Python calls, gathered chatty Python
calls, and one coarse runtime batch against the same six-property B/IP
workload. It does not compare revisions and does not itself support a
performance claim.

The source checkout is explicit and must normally be clean:

```sh
BACNET_BENCH_SOURCE_CONTEXT=/absolute/path/to/immutable/checkout \
  benchmarks/python/run-runtime-ab.sh
```

`BACNET_BENCH_ALLOW_DIRTY=1` exists only for harness validation. Evidence from
a dirty source must not be used for a performance claim. The runner pins the
default build images to Rust 1.97.1 and CPython 3.11, records the source commit,
ref, archive digest, wheel digest, image IDs, tool versions, harness digest,
resolved Compose file, raw latency samples, RSS samples, runtime health, FD
classification, and cleanup assertions.

Before timed trials, a separate correctness-only arm exercises native submit,
cancel, terminal result delivery, health stabilization, and shutdown. Every
timed workflow checks exact values and ordering. Trials alternate arm order;
throughput coefficients of variation above 5% mark the result inconclusive.

The raw result is written incrementally to
`target/runtime-ab/runtime-batch-ab.json`. Validate it with
`benchmarks/schema/runtime-batch-ab.schema.json`. Do not publish a performance
claim until the immutable-source run, schema validation, correctness evidence,
variance, and limitations have been reviewed together.
