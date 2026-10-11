# Embedded BACnet/IP↔MS/TP Router Workplan

Status: active local-fork implementation plan  
Scope: portable library components only; firmware integration and hardware
validation remain downstream responsibilities.

## Architecture decision

The hosted `bacnet-network` router remains the runtime implementation for
Tokio transports. It cannot be used directly on ESP-IDF because it owns Tokio
channels/tasks, `std` clocks and hosted `bacnet-transport` ports. Embedded
routing is therefore implemented in the independent `bacnet-router-core`
crate, following the synchronous, caller-clock pattern already used by
`bacnet-mstp-core`.

The core must not own sockets, UARTs, executors or unbounded queues. A platform
adapter provides ingress metadata and implements synchronous egress admission.
`AdmissionOutcome::AcceptedOwned` means the actual data-link queue copied or
took ownership of the NPDU before returning. Router scheduling alone is not
success.

## Fixed invariants

- Validate the complete NPDU length after routing-header rewriting against the
  selected port's `max_npdu_len`; never use APDU or ingress length as a proxy.
- Keep configured direct-route capacity independent of learned-route capacity.
  Discovery traffic cannot evict a configured network.
- Preserve immediate UDP peer, actual local destination/group provenance and
  logical Forwarded-NPDU origin as distinct values.
- Keep network-control capacity distinct from ordinary data capacity in every
  concrete adapter.
- Reject malformed control payloads before any routing-table mutation.
- Never truncate or fragment an NPDU in the router.
- Standard MS/TP remains capped at a 501-byte NPDU until the Extended MS/TP
  work item below is complete end to end.

## Implementation phases

### R0 — Portable boundary and direct forwarding (implemented)

- [x] Add allocation-free `bacnet-router-core`.
- [x] Add strict borrowed BVLL decode and caller-buffered encode.
- [x] Require actual UDP destination metadata and preserve immediate/logical
  source identities.
- [x] Add independently bounded direct and learned route storage.
- [x] Add direct-network NPDU rewrite with priority preservation.
- [x] Add per-port final-NPDU MTU validation.
- [x] Add synchronous owned-admission results and diagnostic counters.
- [x] Prove the B/IP→standard-MS/TP 501-byte accept / 502-byte reject boundary.

This phase is foundation work, not a claim that the embedded router is
complete.

### R1 — Complete unicast routing

- [ ] Forward through learned next hops and decrement hop count exactly once.
- [ ] Reject hop-count exhaustion and unreachable/busy routes deterministically.
- [ ] Preserve standard and proprietary network-message type/vendor fields.
- [ ] Generate and route Reject-Message-To-Network, including reason 4 for a
  final NPDU that exceeds egress capability.
- [ ] Retain enough ingress context to report admission refusal to the source.

### R2 — Router discovery and table control

- [ ] Implement strict Who-Is-Router-To-Network handling.
- [ ] Implement I-Am-Router-To-Network learning and announcements.
- [ ] Implement Initialize-Routing-Table and ACK parsing transactionally.
- [ ] Implement Router-Busy/Router-Available with next-hop ownership checks.
- [ ] Add route aging, explicit control commands and detached snapshots.

### R3 — Routed broadcasts and loop safety

- [ ] Add configurable remote/global broadcast forwarding policy.
- [ ] Add bounded, expiring echo suppression keyed by canonical origin and
  destination semantics, with hop count excluded from the loop identity.
- [ ] Reserve control admission under data saturation.
- [ ] Test partial fanout admission, cache capacity/expiry and legitimate
  retransmission after expiry.

### R4 — Concrete adapters and composition

- [ ] Add the library-side ESP-IDF UDP contract around datagram source,
  destination, truncation and group metadata; firmware owns actual socket APIs.
- [ ] Bridge router admission directly to `bacnet-mstp-core` queues while
  preserving NPDU priority and exact queue rejection reason.
- [ ] Add composed B/IP→router→MS/TP and reverse-direction deterministic tests.
- [ ] Add bare-target dependency and file-size validation for the new crate.

### R5 — Diagnostics, operational control and hosted convergence

- [ ] Expose per-port accepted, full, oversize, expired, malformed, duplicate,
  policy-drop and transport-failure counters.
- [ ] Add route clear/refresh, port disable and announcement controls.
- [ ] Share wire vectors with the hosted router and decide whether the hosted
  runtime should adapt to the core without changing its public API.
- [ ] Benchmark bounded admission latency and memory across configured profiles.

## Extended MS/TP prerequisite

Frame types `0x20` (Data Expecting Reply) and `0x21` (Data Not Expecting Reply)
use COBS and CRC-32K and are currently rejected deliberately. Types
`0x22..=0x7f` share the extended framing range but must not automatically be
treated as BACnet NPDU frames.

This is a separate local-fork work item and blocks any broad modern-router
claim. It must cover:

1. independent COBS/XOR and CRC-32K wire vectors and malformed-input tests;
2. caller-buffered or in-place extended decode (the immutable borrowed decoder
   cannot return transformed COBS payload bytes);
3. streaming resynchronization and fuzz coverage;
4. explicit larger queue/action/scratch profiles throughout
   `bacnet-mstp-core`, including frame-type selection;
5. hosted compatibility adapters and timeout calculations;
6. 502-byte and 1497-byte end-to-end queue→frame→decode→delivery tests; and
7. measured memory costs and per-port capability reporting.

Simply widening `MAX_NPDU_LEN` or adding codec support is insufficient: current
core action rings and transmission scratch storage remain standard-frame sized.

## Verification gates

- `cargo test -p bacnet-router-core --locked`
- `cargo clippy -p bacnet-router-core --all-targets -- -D warnings`
- bare-target checks for `thumbv7em-none-eabi` and
  `riscv32imac-unknown-none-elf`
- embedded dependency-tree check proving no hosted runtime dependencies
- repository 700-line Rust source-file cap
- existing `bacnet-network`, `bacnet-mstp-codec` and `bacnet-mstp-core` focused
  regression suites after each integration slice

Normative caveat: the configured `_spec/` Standard 135-2020 PDF is absent from
this checkout. Existing Clause 6 ledger anchors and hosted tests inform the
work, but exact clause wording and Extended MS/TP length rules must be checked
against the Standard before raising conformance status.
