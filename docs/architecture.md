# Architecture Guide

This document explains how the rusty-bacnet crates fit together, how data flows through the stack, and how the major subsystems work.

## Crate layout and selected dependencies

This layout groups responsibilities; it is not a complete Cargo dependency graph.
The arrows below mean “depends on.” Lower-level dependencies and feature edges
are omitted; each crate's `Cargo.toml` is the exact dependency authority.

```text
Foundations
  bacnet-types          Enums, primitives, error types (no I/O; no_std + alloc)
  bacnet-encoding       ASN.1 tags, APDU/NPDU codecs, value encoding (no_std + alloc)
  bacnet-services       Service request/response structures (no_std + alloc)
  bacnet-mstp-codec     MS/TP frame codec (no_std; by default no allocator, no deps)
  bacnet-mstp-core      MS/TP master state machine (no_std; by default no allocator)
  bacnet-bip-codec      B/IP and B/IPv6 BVLL codec (no_std; by default no allocator, no deps)
  bacnet-bip-core       B/IP behaviour: BBMD tables and forwarding (no_std; no allocator)
  bacnet-transport      Data-link transports and framing (B/IP, B/IPv6 BVLL via bacnet-bip-codec;
                        the B/IP BBMD via bacnet-bip-core)
  bacnet-network        Network layer, BACnetRouter, RouterTable
  bacnet-objects        BACnetObject, ObjectDatabase, object implementations

Runtime and application roles
  bacnet-endpoint-core  Private lifecycle, ingress, egress, coordination
  bacnet-client         Async requester, transactions, discovery
  bacnet-server         Full server dispatch, COV, events, scheduling
  bacnet-endpoint       Public shared owner composing bounded sibling roles

Selected direct dependencies
  bacnet-endpoint -> bacnet-client, bacnet-server, bacnet-endpoint-core,
                     bacnet-network, bacnet-objects
  bacnet-cli     -> bacnet-client              (CLI application)
  rusty-bacnet   -> bacnet-client, bacnet-server, bacnet-endpoint
                                              (PyO3 bindings)
```

`bacnet-client`, `bacnet-server` and `bacnet-endpoint` are all workspace
`default-members`, along with the foundational crates, endpoint-core,
integration tests, the MS/TP bus simulator and benchmarks. The CLI
(`bacnet-cli`) and PyO3 binding (`rusty-bacnet`) are excluded from default
builds: the CLI pulls in heavier application dependencies, and the Python
extension needs its native Python build context. They remain workspace
members and can be selected explicitly. So does the unpublished firmware link
test, `firmware/link`, which links the MS/TP stack (`bacnet-mstp-codec`,
`bacnet-mstp-core`) and the B/IP and B/IPv6 codec (`bacnet-bip-codec`) into
bare-metal images without an allocator and checks their size
([CI](ci.md#firmware-link-test)).

## Packet Flow

### Inbound (receiving a BACnet request)

```
Physical network (UDP socket / WebSocket / serial port)
    |
    v
TransportPort::start() -> mpsc::Receiver<ReceivedNpdu>
    |  Decodes data-link framing (BVLL for BIP and BIP6, BVLC-SC for SC, MS/TP frames)
    |  Extracts NPDU bytes + source MAC + raw link-layer group provenance + attributes
    v
NetworkLayer::start() -> mpsc::Receiver<ReceivedApdu>
    |  Decodes NPDU header (version, control, DNET/DADR/SNET/SADR)
    |  Local network controls -> owner control intake -> bounded Number worker
    |  Other raw controls retain their consumer (for example client Reject correlation)
    |  Filters: drops messages not for this device (wrong DNET)
    |  Extracts APDU bytes + source addressing + raw/effective group facts + attributes
    v
Optional private EndpointIngress classifier / Client dispatch task / Server dispatch task
    |  Decodes APDU header (PDU type, service choice, invoke ID)
    |  Routes to appropriate handler
    v
Service handler (e.g., handle_read_property)
    |  Decodes service request from APDU payload
    |  Reads/writes ObjectDatabase
    |  Encodes response
    v
NetworkLayer::send_apdu() -> TransportPort::send_unicast()
    |  Encodes NPDU header + APDU payload
    |  Sends via transport
    v
Physical network
```

### Multi-network routing

When `BACnetRouter` is used (multi-transport gateway):

```
Transport A (BIP, network 1)  ─┐
Transport B (SC, network 2)   ─┤──> BACnetRouter
Transport C (MS/TP, network 3) ─┤      |
Loopback (local client/server) ─┘      |
                                        v
                                  RouterTable lookup
                                        |
                                  Forward NPDU to correct transport
```

The router receives NPDUs from all transports, checks the destination network number in the NPDU header, and forwards to the appropriate transport. Messages for the local device (DNET matches a loopback port) are delivered to the client/server.

Data attributes are carried on `ReceivedNpdu` and `ReceivedApdu`, and attribute-aware send helpers are available on `TransportPort` and `NetworkLayer`. `ReceivedApdu.link_layer_group` preserves whether the incoming data-link destination was a group address, while `ReceivedApdu.is_group` describes the effective BACnet network destination; a routed unicast can therefore have `link_layer_group == true` and `is_group == false`. `ReceivedApdu.global_broadcast` marks an NPDU addressed to the global broadcast network (DNET 65535), which `is_group` alone does not tell apart from a local or one-network broadcast. BACnet/SC maps every inbound Annex AB Data Option to these attributes and maps outbound attributes back to SC Data Options. Unknown Data Options do not block NPDU delivery. For received Encapsulated-NPDUs, an unsupported Must Understand Destination Option returns a BVLC-Result NAK with the original option marker to the unicast source through the hub and drops broadcast without a result. This message-level NAK does not close the source connection. A Destination Option without Must Understand does not block delivery. The current `DataAttribute.must_understand` field stores the Data Option bit-6 value, but Addendum 135-2020cf Every Segment behavior is not implemented. The router preserves inbound data attributes when forwarding unicast or broadcast NPDUs across attribute-capable transports, while data links that do not support attributes expose an empty list on receive and ignore attributes on send.

## Transport Abstraction

All transports implement the `TransportPort` trait:

```rust
pub trait TransportPort: Send + Sync {
    fn start(&mut self) -> impl Future<Output = Result<mpsc::Receiver<ReceivedNpdu>, Error>> + Send;
    fn stop(&mut self) -> impl Future<Output = Result<(), Error>> + Send;
    fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> impl Future<Output = Result<(), Error>> + Send;
    fn send_broadcast(&self, npdu: &[u8]) -> impl Future<Output = Result<(), Error>> + Send;
    fn local_mac(&self) -> &[u8];
    fn local_receive_apdu_capacity(&self) -> u16; // stable local declaration
    fn egress_apdu_limit(&self) -> u16; // current outgoing path limit
    fn supports_local_nonrouter_number_controls(&self) -> bool; // opt-in, default false
    fn bip_port(&self) -> Option<BipPort>; // registration metadata: endpoint and B/IP mode
}
```

`TransportPort` owns data-link framing and link-specific controls. `NetworkLayer` owns NPDU addressing and APDU delivery forms. The private `bacnet-endpoint-core` runtime can own one network lifecycle and expose bounded ingress and network-service egress to application-role adapters; those role handles cannot start or stop the network or transport. The public `bacnet-endpoint` crate composes sibling requester and bounded responder roles on that private foundation. One `EndpointSession` owns one B/IP, SC or MS/TP transport; this is not a multi-link router or full `bacnet-server` responder replacement. See [endpoint scope](rust-api.md#bacnet-endpoint).

Local nonrouter Number controls take a separate bounded path: one serial state owner per standalone client, full server or shared endpoint consumes eligible parsed controls without blocking independent APDU dispatch. Raw network controls remain available for other consumers, including routed Reject correlation. A client or unregistered owner starts UNKNOWN on an opted-in transport. Only an explicitly registered B/IP receiving-port object, in any B/IP mode, supplies configured number authority; an unrelated database declaration cannot supply it. The capability is separate from registration metadata and from multiport/router behavior. See [Number controls and lifecycle](rust-api.md#local-network-number-controls).

MAC address format varies by transport:
- **BIP**: 6 bytes (4-byte IPv4 + 2-byte port, big-endian)
- **BIP6**: 3 bytes (the node's VMAC, Annex U.1.3); the transport maps VMACs to
  IPv6 addresses and ports itself, resolving unknown ones (U.5)
- **MS/TP**: 1 byte (station address 0-254)
- **BACnet/SC**: 6 bytes (VMAC)
- **Ethernet**: 6 bytes (IEEE 802 MAC)
- **Loopback**: arbitrary (synthetic, e.g., `[0x00, 0x01]`)

`AnyTransport<S>` is a type-erased enum wrapping all transport types, enabling mixed-transport routing (e.g., BIP + MS/TP + Loopback on the same router).

### RS-485 Direction Control (MS/TP)

RS-485 is half-duplex — the transceiver's DE/RE pin must be toggled between transmit and receive. The stack supports three modes:

```
                              ┌──────────────────────────┐
USB RS-485 Adapter ──────────>│  TokioSerialPort         │  Auto-direction
(FTDI, CH340, etc.)           │  (no config needed)      │  (hardware handles DE/RE)
                              └──────────────────────────┘

UART + RTS → DE/RE ──────────>│  TokioSerialPort         │  Kernel RS-485
(DE wired to UART RTS pin)    │  .enable_kernel_rs485()  │  (TIOCSRS485 ioctl)
                              └──────────────────────────┘

UART + GPIO → DE/RE ─────────>│  GpioDirectionPort<S>    │  GPIO direction
(Pi hat, GPIO pin for DE)     │  wraps any SerialPort    │  (gpiocdev, serial-gpio feature)
                              └──────────────────────────┘
```

`GpioDirectionPort` wraps a `SerialPort` with transmit-complete `drain()` support and controls a GPIO pin through the Linux character device API (`/dev/gpiochipN`). It asserts DE before writing, waits for drain, then applies any configured transceiver guard interval before returning to RX. Unix `TokioSerialPort` drains through the native serial backend on a blocking worker while retaining exclusive ownership of the stream. Hardware auto-direction and kernel RS-485 add no userspace direction changes; the transport still drains every frame, to learn when it ended.

### The MS/TP transport and the portable core

`MstpTransport` keeps the serial I/O layer above and runs the master node state machine of `bacnet-mstp-core`, with frames from `bacnet-mstp-codec` (re-exported as `bacnet_transport::mstp::codec`). There is no second state machine or codec in the transport. Its task owns the core, the core's receive buffer and a boxed `ActionBuffers`, and acts as the core's adapter:

- **Input.** Every octet a serial read returns goes to `on_octet`, stamped with the time of that read, so the core's own stream decoder (the one the simulator and fuzzers exercise) assembles every frame. A host stamp can only be late, which can only defer this station's sends. T_frame_abort is 100 ms, the most Clause 9.5.3 allows, so a frame whose tail a USB adapter holds back still completes.
- **Deadlines.** Tokio time is mapped onto the core's microsecond clock from the moment the transport started. The task sleeps until `next_deadline` and stamps `on_deadline` with the time it woke, so late service shows in the core's `deadline_misses`.
- **Sending.** Octets already waiting are fed to the core right before `on_transmit_started`; the frame is written only if that returns Ok. It is complete when the port's `drain` returns, and never before its time on the wire at the baud rate (a port without a drain is timed by that alone). Each frame, including this station's own consecutive frames, waits T_turnaround after the last activity.
- **Echo.** `SerialPort::echo` says whether the port hears its own frames. With `Echo::Full { grace }` (`Echo::full()` gives 40 ms, room for USB receive latency; zero is refused at start) the transport hunts for the frame it sent in what it reads. Octets that could begin the echo are held back and dropped once the whole frame has matched; an octet that does not fit passes on to the core with as few of the held octets as it takes, so a stray octet before the echo (a peer's pad octet) passes on and the echo behind it is still dropped, and a peer's frame that only starts like the echo passes on whole. The frame is reported complete at the first octet that passes on, when the echo has matched, or at the end of the grace, always before the core is fed anything read after it. The hunt then goes on: an echo under way is still matched as it continues, one not begun is looked for during one more grace (ended by a timer), and the hunt stops at the next confirmed start (octets held in the probe pass on before the station tries to start) or, while an echo under way is still being matched, after T_frame_abort of silence. Held octets that waited longer than T_frame_abort are dropped as a stale fragment. Mismatches, missing echoes and dropped fragments are counted.
- **Queues.** The core's unicast and broadcast queues (256 NPDUs each) are shared with the send calls behind a short-held mutex, so a full queue or an oversized NPDU is refused at once. NPDUs go out by network priority.
- **Replies.** A Data Expecting Reply for this station goes up with a one-use reply channel; an answer through it goes to `complete_request`. When the core stops waiting it sends Reply Postponed, and the channel is kept in a deferred set: an answer that comes later is queued as an ordinary NPDU to the requester and goes out with the next token (Clause 9.5.6.9). The set holds eight channels, dropping the oldest past that. Each expires at its request's arrival plus the queue's maximum age, and an answer queued before then keeps that expiry in the queue; drops and expiries are counted.

These host-side guarantees are not physical UART timing qualification.

`MstpTransport::with_execution_mode(MstpExecutionMode::DedicatedThread)` opts into
an OS thread with a current-thread Tokio runtime for the MAC loop. The default
`Tokio` mode retains the application-runtime spawn path. Both modes run the same
core, adapter and 64-entry NPDU receive channel. Serial wrappers retain their existing drain
boundary; native blocking drain jobs use the isolated runtime's blocking pool
when called from the dedicated loop. No per-frame bridge or second MAC machine
is introduced. Already-open async serial resources still depend on their original
reactor, which must remain running.

`stop()` cancels the MAC task, waits for isolated runtime teardown and outstanding
blocking work; the core and its queues go with the task, and later sends are refused.
`abort()` and drop request teardown without waiting. Cancellation cannot interrupt
a blocking syscall or undo bytes already accepted by a driver; a stuck backend
can delay shutdown. Fast or efficient execution is not guaranteed or measured
deterministic timing. Neither execution mode qualifies real hardware timing.

Deferred from this thread-isolation subset: RT scheduling policy/priority
(`SCHED_FIFO`), CPU affinity/pinning and observable RT setup results; PREEMPT_RT,
IRQ, mlock and buffer-tuning deployment guidance beyond this note; and on-wire
hardware qualification (#502). These RT APIs are not implemented here, and #501
remains open for its residual RT-policy/affinity and full documentation work.

### The B/IP BBMD and the portable core

A `BipTransport` in BBMD mode keeps its tables in a `BbmdState`, which runs
the BBMD of `bacnet-bip-core`: a `bacnet_bip_core::bbmd::Tables<128, 128>`
holding the BDT and the FDT in fixed arrays, and the decisions over them.
The core decides; the transport does the I/O and keeps the host's policy:

- **In the core** (`no_std`, no allocator, no clock): BDT validation and
  canonical form, the BBMD's own row, foreign-device registration (the
  time-to-live limits, the per-address quota, reserved capacity), expiry
  after the time-to-live plus the 30-second grace period, and where each
  broadcast goes (Annex J.4.5): an Original-Broadcast-NPDU to each peer's
  distribution address (its address ORed with its inverted mask) and the
  foreign devices; a peer's Forwarded-NPDU to the foreign devices and, when
  it came to this BBMD alone, the local subnet (its own BDT row is two-hop,
  it arrived by unicast, and its sender isn't on this subnet at this BBMD's
  UDP port); a
  registered foreign device's Distribute-Broadcast-To-Network to the local
  subnet, the peers and the other foreign devices, or its NAK; the BBMD's own
  broadcasts. Each decision writes its destinations into storage the caller
  passes.
- **In the transport**: the foreign-device source list and registration rate
  limit (the core asks them through its admission check), the
  Delete-FDT-Entry list, the FDT and fan-out counters, BDT persistence, the
  management rate limiter, the fan-out queue and its budgets, and the sockets.

At start the transport asks the host for the mask of the subnet the BBMD's
own address is on, for the last of those conditions; where the host doesn't
report one, a forward from the BBMD's own subnet is rebroadcast.

`BbmdState` maps `std::time::Instant` onto the core's nanosecond clock, with
the state's creation at the middle of its range, so every instant maps
exactly both ways. Each decision takes the state's lock once, copies its
destinations out, and releases it before anything is sent; no await happens
under the lock. A purge task removes lapsed registrations every second, and
each decision skips them either way. Firmware can drive the same
`bacnet_bip_core::bbmd::Tables` directly, sized to fit.

### Testing MS/TP without hardware

`bacnet-mstp-sim`, a test-only crate that is not published, puts several
`bacnet-mstp-core` masters on a simulated RS-485 line and runs them in
virtual time: ten-bit characters at the configured baud rate, collisions,
noise, power loss and slow adapters, with Clause 9 invariants checked after
every event. Its scenarios cover ring formation, rotation, replies, joins and
leaves, token loss and noise, plus randomized fault runs. The crate
documentation describes the model and what it leaves out, such as electrical
effects and propagation delay. The simulator feeds each core octet by octet
from virtual time; the host `MstpTransport` drives the same core from chunked
reads, and its own tests (Clause 9 regressions for #1608 among them) run it
against a scripted peer in paused Tokio time.

## Object Model

Every BACnet object implements the `BACnetObject` trait:

```rust
pub trait BACnetObject: Send + Sync {
    fn object_identifier(&self) -> ObjectIdentifier;
    fn object_name(&self) -> &str;
    fn read_property(&self, property: PropertyIdentifier, array_index: Option<u32>) -> Result<PropertyValue, Error>;
    fn write_property(&mut self, property: PropertyIdentifier, array_index: Option<u32>, value: PropertyValue, priority: Option<u8>) -> Result<(), Error>;
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]>;
    // ... plus COV, intrinsic reporting, scheduling methods
}
```

`ObjectDatabase` stores `Box<dyn BACnetObject>` keyed by `ObjectIdentifier`, with secondary indexes by name (for WhoHas) and by type (for efficient enumeration).

## Concurrency Model

The stack runs on a Tokio multi-threaded runtime.

**Lock ordering** (server): always lock `db` (ObjectDatabase) before `cov_table` (COV subscriptions). With target Audit configured, a DeviceCommunicationControl change and its timer's expiry take the DCC timer slot first and `db` second, to report the change under the slot (#1387); nothing takes the slot while holding `db`. Violating either order risks deadlock.

**Resource exhaustion caps**:
- COV subscriptions: 1,024 max
- BBMD tables: 128 BDT rows and 128 FDT entries
- Objects per database: 10,000
- Router table entries: 256
- Segment receivers: 128 (prevents DoS from abandoned segmented transfers)
- COV in-flight: 255 (matches u8 invoke ID range)
- MS/TP frame buffer: 511 bytes (one standard frame)
- MS/TP queues: 256 pending unicast and 256 pending broadcast NPDUs

**Client APDU retry**: 3 retries by default, invoke ID reused across retries, cleaned up on final timeout.

## Server Engine

The `BACnetServer` spawns several background tasks:

| Task | Purpose | Interval |
|------|---------|----------|
| Dispatch | Receives APDUs, routes to service handlers | Event-driven |
| COV purge | Removes expired COV subscriptions | 60s |
| Fault detection | Evaluates analog objects for over/under-range | 10s |
| Intrinsic reporting | Advances Time_Delay countdowns and fires confirmed transitions via `tick_intrinsic_reporting` | 1s |
| Event enrollment | Evaluates Event Enrollment objects against their monitored properties | 10s |
| Trend log | Records data samples for trend log objects | Per-object interval |
| Schedule tick | Evaluates Schedule objects (exceptions by period and priority, weekly schedule, Schedule_Default, within Effective_Period) against one Device clock frame and Calendar states, and writes changes at Priority_For_Writing; a write that commits to a Schedule runs its pass at once, which also sends on a Present_Value written while out of service and relinquishes slots a change of references or priority left behind; each target's result feeds the Schedule's Reliability | 60s |

The server handles 20+ services including ReadProperty, WriteProperty, ReadPropertyMultiple, WritePropertyMultiple, SubscribeCOV, CreateObject, DeleteObject, DeviceCommunicationControl, GetEventInformation, GetAlarmSummary, LifeSafetyOperation, AtomicReadFile, AtomicWriteFile, TimeSynchronization, and more.

ReinitializeDevice (Clause 16.4) is how a peer asks a device to restart, to
apply changes, or to step through a Clause 19 backup or restore. The server
decodes it and checks the password, then passes a `ReinitializeContext` (the
requested state, the requester's source and provenance, the invoke ID) to the
handler set with `on_reinitialize`, which carries it out with the object
database write-locked; an error the handler returns is sent in place of the
SimpleACK. Without a handler, or for a state Clause 16.4 does not define, the
request is refused with `SERVICES / SERVICE_REQUEST_DENIED`. Password failures
and malformed-request errors keep their precedence. The SimpleACK leaves only
after the handler returns, so a handler schedules any restart for after the
reply and hands slow work to a task. The mutation policy doesn't cover this
service; the handler restricts sources through its context. A handler panic
or a returned Reject is answered `SERVICES / OTHER`, since by then the request
can no longer be rejected (Clause 20.1.8). Endpoint sessions run the same
handler through their narrow responder.
