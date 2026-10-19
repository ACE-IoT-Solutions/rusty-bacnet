# Rusty BACnet benchmarks

This page reports what rusty-bacnet 0.13.0 does on one machine, measured on
2026-10-13: how fast it encodes and decodes, how long a request takes on each
transport, how many requests a server answers, what routing costs, how close
MS/TP gets to the wire, and how small the embedded builds are. It also says
how each number was measured, so you can repeat it, and where the numbers are
weak.

## Contents

1. [Summary](#1-summary)
2. [Test environment](#2-test-environment)
3. [Encoding and the portable codecs](#3-encoding-and-the-portable-codecs)
4. [Transports](#4-transports)
5. [Routing](#5-routing)
6. [Scale and stress](#6-scale-and-stress)
7. [Docker cross-network results](#7-docker-cross-network-results)
8. [Embedded footprint](#8-embedded-footprint)
9. [Python bindings](#9-python-bindings)
10. [Guidance for future work](#10-guidance-for-future-work)
11. [How to reproduce](#11-how-to-reproduce)

## 1. Summary

These are the figures most projects ask about. Every one comes from a single
run on an Apple M5 laptop over the loopback interface, so treat them as what
the stack itself costs, not as what a building network will show: a real
network adds its own delay, and a real field bus is far slower than the
host.

- **Request latency on IP transports.** One ReadProperty round trip, client
  to server and back, takes a median of 35 microseconds over BACnet/IP
  (B/IP, BACnet over UDP and IPv4), 31 microseconds over BACnet/IPv6
  (B/IPv6), and 54 microseconds over BACnet Secure Connect (BACnet/SC,
  BACnet over Transport Layer Security (TLS) WebSockets through a hub)
  with mutual TLS
  ([section 4](#4-transports)).
- **Server throughput.** One B/IP server answered about 74,000
  ReadProperty requests per second from 10 concurrent clients, and kept
  65,000 to 70,000 requests per second from 25 to 100 clients. A weighted
  mix of reads, writes, ReadPropertyMultiple, change-of-value (COV)
  subscriptions and Who-Is reached 76,800 operations per second with no
  errors ([section 6](#6-scale-and-stress)). In a Linux container on the
  same laptop, with 4 of its processors, the same scenarios ran 20% to 70%
  faster: up to 117,000 requests per second
  ([section 7.2](#72-stress-scenarios-inside-a-container)).
- **Database size does not slow reads.** ReadProperty latency stayed at a
  median of 33 to 34 microseconds from 100 to 10,000 objects; resident
  memory grew by about 1.25 kilobytes per Analog Input object
  ([section 6.2](#62-object-database-size)).
- **Routing.** A request routed through `BACnetRouter` from one B/IP
  network to another took a median of 49 microseconds against 31 direct,
  so the router hop adds about 18 microseconds on loopback. Under load the
  routed path delivered 57,000 requests per second, 84% of the direct
  path's rate ([section 5](#5-routing)).
- **MS/TP efficiency.** Over a simulated RS-485 line, MS/TP (BACnet's
  Master-Slave/Token-Passing serial link) reached 90% to 99% of the wire
  limit for large replies at every baud rate from 9,600 to 115,200. For a
  small reply the fixed host overhead of about 7 milliseconds per request
  matters more: 91% of the wire limit at 9,600 baud, 53% at 115,200
  ([section 4.4](#44-mstp)).
- **Embedded footprint.** Without an allocator, the MS/TP master node links
  into 18.4 kilobytes of flash and 5.5 kilobytes of static RAM on a
  Cortex-M3, the B/IP codec into 10.9 kilobytes of flash, and a two-port
  B/IP-to-MS/TP router into 45.1 kilobytes of flash and 15.5 kilobytes of
  RAM ([section 8](#8-embedded-footprint)).
- **Python.** A request through the Python package takes 70 to 100
  microseconds, and one Python client reaches about 24,000 requests per
  second with 10 in flight ([section 9](#9-python-bindings)).
- **Resource limits by design.** A server accepts 64 COV subscriptions per
  subscriber by default, 16 confirmed requests in flight per client, a
  ReadPropertyMultiple answer of at most 256 results, and a BBMD (the B/IP
  broadcast relay) admits 16 registrations per source address per second.
  Some stress steps go past these limits on purpose; each place is noted
  where it shows.

## 2. Test environment

| Item | Value |
| --- | --- |
| Machine | MacBook Pro (Mac17,2), Apple M5, 10 cores (4 performance, 6 efficiency), 16 gigabytes of memory |
| Operating system | macOS 27.2 (build 26B5091g) |
| Rust | rustc 1.99.0 (b940084d7 2026-09-28), release builds; Criterion suites use the `bench` profile |
| TLS provider | rustls with aws-lc-rs |
| Commit | The stack as released: `dev` at `497695c0` (the 0.13.0 release merge). The benchmark harness: commit `deaeb745`, which adds the scenarios and fixes this report describes and changes nothing outside `benchmarks/` but `Cargo.lock` |
| Docker | Docker Desktop 29.8.2, Linux virtual machine with 4 processors and 4.8 gigabytes of memory; Alpine Linux 3.21 images with statically linked release binaries |
| Python | CPython 3.13.15, the `rusty-bacnet` package built with `maturin develop --release` |
| Date | 2026-10-13 |

How the numbers were taken:

- Every suite ran by itself, one after another, on an otherwise idle
  machine. Running two suites at once makes both numbers wrong.
- All network traffic stays on the loopback interface (127.0.0.1 or ::1).
  Loopback has no wire delay and almost no loss, so it flatters every
  network figure: it shows the cost of the stack, not of a network.
- Criterion figures are the median of Criterion's estimate over its default
  100 samples, after its warm-up. The stress figures are a single run of
  each step, usually 10 seconds long; they vary by a few percent between
  runs.
- A laptop changes its clock speed with temperature and load. The
  efficiency cores also take some threads. Expect a server-class machine to
  give steadier numbers, not necessarily faster ones.
- The previous edition of this page measured 0.8.0 on a machine of the same
  model under macOS 26.4 and Rust 1.94. Those numbers are not comparable
  with these; [section 10](#10-guidance-for-future-work) notes where they
  differ most.

## 3. Encoding and the portable codecs

These benchmarks run no input or output: they measure the processor time
of one operation. The times are on the M5; a microcontroller runs the same
code tens to hundreds of times slower.

### 3.1 Application and network layer encoding

Suite `encoding`. The NPDU is the network protocol data unit, the APDU the
application protocol data unit.

| Operation | Median time (nanoseconds) |
| --- | --- |
| Encode a ReadProperty request (service parameters only) | 22.6 |
| Decode a ReadProperty request | 19.9 |
| Encode an NPDU carrying 100 bytes | 15.5 |
| Decode an NPDU carrying 100 bytes | 11.2 |
| Encode a confirmed-request APDU header | 17.7 |
| Decode a confirmed-request APDU header | 12.8 |
| Encode a whole ReadProperty request: service, APDU and NPDU | 68.7 |

A complete request takes about 70 nanoseconds to build, so encoding is a
fraction of a percent of the 35-microsecond round trip in
[section 4](#4-transports).

### 3.2 Portable `no_std` codecs and the router core

Suite `codecs`, new in this edition. These crates build for
microcontrollers without an allocator; the same code runs here on the host.
CRC stands for cyclic redundancy check. A routed NPDU names its destination
network and address (DNET and DADR); a router adds the source network and
address (SNET and SADR) when it forwards one.

| Codec | Operation | Median time (nanoseconds) |
| --- | --- | --- |
| B/IP BACnet Virtual Link Layer (BVLL) | Encode an Original-Unicast-NPDU of 13 bytes | 4.2 |
| B/IP BVLL | Decode it | 3.7 |
| B/IP BVLL | Encode a Forwarded-NPDU of 1,497 bytes | 15.0 |
| B/IP BVLL | Decode it (the NPDU is borrowed, not copied) | 3.7 |
| MS/TP frame | Encode a Token frame | 4.4 |
| MS/TP frame | Encode a standard data frame of 501 bytes | 845 |
| MS/TP frame | Decode it | 821 |
| MS/TP frame | Encode an extended data frame of 1,497 bytes | 5,982 |
| MS/TP frame | Decode it (in place) | 4,980 |
| MS/TP frame | CRC-16 over 501 bytes (the standard frame's data check) | 825 |
| MS/TP frame | CRC-32K over 1,497 bytes (the extended frame's data check) | 4,719 |
| Router core | Parse a routed ReadProperty NPDU (DNET, 6-byte DADR) | 6.8 |
| Router core | Parse it and write the final-hop rewrite (DNET dropped, SNET and SADR added) | 9.0 |
| `bacnet-encoding` | Decode the same routed NPDU into its owned form, for comparison | 16.5 |

What the table shows:

- The B/IP codec costs a few nanoseconds whatever the size, because decoding
  only checks the header and borrows the payload.
- The MS/TP frame cost is almost all checksum: the CRC-16 runs at about
  1.6 nanoseconds per byte and the CRC-32K at about 3.2. The default
  CRC-32K uses a 16-entry table to save flash; the `crc32k-table` feature
  trades about 1 kilobyte more flash for half the table lookups. Even the
  slowest case is far below the frame's time on the wire: a 1,497-byte
  extended frame takes 130 milliseconds to send at 115,200 baud.
- The router core's borrowed NPDU view parses a routed NPDU in about 40% of
  the time `bacnet-encoding`'s owned decoder takes, and rewrites it for the
  next hop in 9 nanoseconds.

### 3.3 ReadRange of a full Trend Log

Suite `read_range`. The server's cost to answer one ReadRange for 100
records from the middle of a full Trend Log, by request kind and log size.
The page is the same size at every log size, so growth with the log is the
cost of finding the window.

| Request kind | Median time, 10,000-record log (microseconds) | Median time, 100,000-record log (microseconds) |
| --- | --- | --- |
| By position | 7.4 | 7.5 |
| By sequence number | 8.8 | 7.4 |
| By time | 7.5 | 7.6 |
| By time, in a log whose clock stepped back once | 31.0 | 152.8 |

Position, sequence and time lookups cost the same at 10 times the log
size. A log whose clock stepped back can't be searched by halving, so the
time lookup scans it, and grows with the log: still under a fifth of a
millisecond at 100,000 records.

## 4. Transports

### 4.1 How these were measured

Suites `bip_latency`, `bip_throughput`, `bip6_latency`, `bip6_throughput`,
`sc_mtls_latency` and `sc_mtls_throughput`. One client and one server in the
same process, over loopback. The server holds a Device, 10 Analog Inputs,
an Analog Output and a Binary Value.

- **Latency** is one request and its answer: ReadProperty of an Analog
  Input's Present_Value, WriteProperty of an Analog Output's
  Present_Value, and ReadPropertyMultiple of 10 Present_Values.
- **Throughput** sends 10, 100 or 1,000 requests one after another, each
  waiting for its answer, and reports the batch's time. It is sequential,
  so it is close to the inverse of latency; for concurrent clients, see
  [section 6.1](#61-concurrent-clients).
- BACnet/SC runs through a hub in the same process, with mutual TLS: the
  hub and the node each check the other's certificate. The older
  server-authentication-only mode has been removed, so there is no
  separate figure for it.

### 4.2 Latency

| Transport | ReadProperty median (microseconds) | WriteProperty median (microseconds) | ReadPropertyMultiple of 10 objects, median (microseconds) |
| --- | --- | --- | --- |
| B/IP | 35.5 | 34.9 | 39.8 |
| B/IPv6 | 31.1 | 33.2 | 37.2 |
| BACnet/SC with mutual TLS | 54.0 | 54.5 | 60.6 |

ReadPropertyMultiple of 10 objects costs only 4 to 6 microseconds more
than one ReadProperty: the round trip, not the property reads, is the cost.

### 4.3 Sequential throughput

The rate is the batch size divided by the batch's median time.

| Transport | ReadProperty, 1,000 in a row (requests per second) | WriteProperty, 1,000 in a row (requests per second) | Median time for 1,000 ReadProperty requests (milliseconds) |
| --- | --- | --- | --- |
| B/IP | 29,300 | 28,600 | 34.2 |
| B/IPv6 | 23,600 (30,200 for batches of 100) | 30,300 | 42.4 |
| BACnet/SC with mutual TLS | 18,300 | 18,400 | 54.5 |

The B/IPv6 ReadProperty batch of 1,000 was noisy (its 95% confidence
interval ran from 39.4 to 46.5 milliseconds); its batches of 100 and its
WriteProperty batches ran at about 30,000 requests per second, so treat
30,000 as typical.

### 4.4 MS/TP

MS/TP runs over RS-485 serial lines at low baud rates, where the wire, not
the processor, limits throughput. The new `stress-test mstp` scenario
measures how close the stack gets to that limit.

**Method.** A client station (MAC address 0) and a server station (MAC
address 1), the only two masters on the line, run the real MS/TP transport
over a simulated RS-485 line (`SimWire` in `benchmarks/src/stress/mstp.rs`).
The line hands each frame's bytes to the other station no sooner than a
serial port's UART (its transmitter hardware) sending them at the baud
rate would, in chunks of about 4
milliseconds, and counts every byte. The client sends one ReadProperty
after another for 10 seconds (20 for the largest reply) at each baud rate.
Three reply sizes:

- **Small:** an Analog Input's Present_Value. Request and reply frames
  together are 52 bytes.
- **Standard:** the Device's Object_List of 92 objects. Request and reply
  frames together are 507 bytes, near the most a standard frame carries.
- **Extended:** the Device's Object_List of 289 objects, over extended
  frames. Request and reply frames together are 1,496 bytes: the reply's
  APDU is about 1,460 bytes, near the 1,476-byte limit.

**The wire limit** is the rate the line could carry the same frames at if
nothing else took time: per request, the request and reply data frames,
one Token frame each way (8 bytes each), and the turnaround of 40 bit times
before each of the four frames, all at 10 bits per byte (start bit, 8 data
bits, stop bit). Every request in every run took exactly those four frames,
so the stack sent no extra frames; the gap between the measured rate and
the limit is time the stations spent between frames.

| Baud rate | Reply | Requests per second | Wire limit (requests per second) | Share of the wire limit | Median latency (milliseconds) | 99th percentile latency (milliseconds) |
| --- | --- | --- | --- | --- | --- | --- |
| 9,600 | Small | 10.4 | 11.4 | 91% | 96.1 | 98.8 |
| 38,400 | Small | 34.4 | 45.7 | 75% | 29.0 | 31.2 |
| 76,800 | Small | 53.2 | 91.4 | 58% | 18.9 | 20.7 |
| 115,200 | Small | 72.7 | 137.1 | 53% | 14.0 | 16.5 |
| 9,600 | Standard | 1.8 | 1.8 | 98% | 570.4 | 572.9 |
| 38,400 | Standard | 6.7 | 7.1 | 95% | 148.4 | 152.6 |
| 76,800 | Standard | 12.8 | 14.2 | 90% | 78.1 | 80.8 |
| 115,200 | Standard | 19.2 | 21.4 | 90% | 51.3 | 55.0 |
| 9,600 | Extended | 0.6 | 0.6 | 99% | 1,600.5 | 1,604.6 |
| 38,400 | Extended | 2.5 | 2.5 | 98% | 405.8 | 408.1 |
| 76,800 | Extended | 4.8 | 5.0 | 96% | 207.4 | 210.3 |
| 115,200 | Extended | 7.1 | 7.5 | 95% | 139.9 | 143.2 |

What it shows:

- Each request spends a roughly fixed 5 to 9 milliseconds beyond its wire
  time, about 1.5 to 2 milliseconds per frame. That is mostly the host's
  timer resolution: Tokio's timers wake on whole milliseconds, and the
  stations wait on a timer before every frame they send. For large replies
  the overhead is small next to the wire time, so the stack reaches 90% to
  99% of the limit. For small replies at high baud rates the wire time
  shrinks to 7 milliseconds and the overhead is half the total.
- One client gets no more from the line by sending concurrently: with one
  information frame per token (the default `max_info_frames` of 1), the
  client sends one request per token rotation
  ([section 5](#5-routing) shows the same through the router).
- The simulated line has no noise, no adapter latency and no other
  stations. A USB serial adapter adds several milliseconds of receive
  latency per frame (16 milliseconds by default on FTDI parts), so real
  hardware will sit further below the limit than this.

### 4.5 Comparison

| Transport | ReadProperty median latency (microseconds) | Sequential ReadProperty requests per second | Most requests per second measured with concurrent clients |
| --- | --- | --- | --- |
| B/IP | 35.5 | 29,300 | 75,700 (10 clients) |
| B/IPv6 | 31.1 | about 30,000 | not measured |
| BACnet/SC with mutual TLS | 54.0 | 18,300 | not measured |
| MS/TP at 115,200 baud, small reply | 14,000 | 72.7 | 72.7 (one line, see above) |
| MS/TP at 38,400 baud, small reply | 29,000 | 34.4 | 34.4 |

BACnet/SC costs about 50% more per request than B/IP: the TLS record
layer and the hub's extra hop. An MS/TP request takes several hundred
times as long as one over IP because the serial line is that much slower;
the stack's own share of an MS/TP request is a few milliseconds.

## 5. Routing

The `router` scenario of `stress-test` now routes real traffic through
`BACnetRouter` (`--path bip` and `--path mstp`); `--path direct` keeps the
same-network baseline. Every routed request names network 2 and the
server's MAC address in its NPDU, so it takes the router's forwarding path
both ways: the router adds the source network and address on the way in
and strips the destination on the final hop.

**B/IP to B/IP.** Clients on B/IP network 1, the server on B/IP network 2,
the router between them, all on loopback. Each client sends ReadProperty
requests one after another for 10 seconds at each step.

| Concurrent clients | Direct: requests per second | Direct: median latency (microseconds) | Direct: 99th percentile latency (microseconds) | Routed: requests per second | Routed: median latency (microseconds) | Routed: 99th percentile latency (microseconds) | Routed rate as a share of direct |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 30,623 | 31 | 44 | 19,366 | 49 | 72 | 63% |
| 5 | 70,724 | 68 | 124 | 42,187 | 113 | 208 | 60% |
| 10 | 75,695 | 128 | 231 | 46,973 | 206 | 370 | 62% |
| 25 | 70,613 | 350 | 599 | 55,874 | 435 | 727 | 79% |
| 50 | 67,883 | 723 | 1,297 | 57,154 | 836 | 1,462 | 84% |

The router hop adds about 18 microseconds to an idle request: one more UDP
datagram each way through the loopback interface (four per request instead
of two), and the router's receive, table lookup, rewrite and send. Under load the router is a
separate set of tasks working in parallel with the server, so the gap
narrows to 16% at 50 clients. No request failed on either path.

**B/IP to MS/TP.** Clients on B/IP network 1, the server on an MS/TP
network at 115,200 baud behind the router (the router's MS/TP port is
station 0, the server station 1, over the simulated line of
[section 4.4](#44-mstp)).

| Concurrent clients | Requests per second | Median latency (milliseconds) | 99th percentile latency (milliseconds) |
| --- | --- | --- | --- |
| 1 | 68 | 14.7 | 18.2 |
| 5 | 67 | 75.0 | 80.6 |

One client through the router got 68 requests per second against 72.7 for
an MS/TP client on the line itself, so the B/IP side and the router add
about 0.7 milliseconds to a request that the serial line already holds for
14. More clients only queue: the line carries one request per token
rotation, and each client's latency grows with the queue.

## 6. Scale and stress

The `stress-test` binary runs these. All are on B/IP over loopback, in one
process, with clients doing requests one after another as fast as answers
come back; `stress-test clients` can also run over B/IPv6 or BACnet/SC.
Resident memory is the whole process's, read from the operating system, so
it includes the clients.

Each scenario prints one line per step, then a table of every step and a
summary of all of them: the requests that succeeded and failed in all, the
rate over all the steps' measured time, and the fastest step. Its JSON
output holds the same, with each step's counts and measured time in the
degradation curve. Before
[#1714](https://github.com/jscott3201/rusty-bacnet/issues/1714) the
summary came from the last step alone, and its count of successful
requests was that step's rate times 10 seconds, whatever the duration; the
per-step figures below were not affected.

### 6.1 Concurrent clients

`stress-test clients`: N clients reading one Analog Input's Present_Value
from a server with 100 Analog Inputs, 10 seconds per step. The table is
B/IP. `--transport bip6` runs the same over B/IPv6 on the IPv6 loopback
address, and `--transport sc` over BACnet/SC with mutual TLS, every client
and the server connected to a hub in the same process; this report has no
figures for those two yet.

Since this run, every client connects before a step's clock starts and
records its latencies in its own histogram. Before, each client connected
inside the 10 seconds and all of them shared one locked histogram, which
may have held the rates with many clients a little low; re-run this table
at the next refresh.

| Concurrent clients | Requests per second | Median latency (microseconds) | 99th percentile latency (microseconds) | Failed requests |
| --- | --- | --- | --- | --- |
| 1 | 30,502 | 32 | 44 | 0 |
| 5 | 70,884 | 68 | 122 | 0 |
| 10 | 73,750 | 132 | 236 | 0 |
| 25 | 69,895 | 353 | 611 | 0 |
| 50 | 65,016 | 749 | 1,366 | 0 |
| 100 | 67,240 | 1,199 | 2,281 | 380 (0.06%) |

Throughput peaks between 5 and 10 clients and holds within 10% of the peak
up to 100; beyond that, more clients only lengthen the queue, and latency
grows in proportion. At 100 clients 380 of 672,778 requests timed out.
Peak resident memory was 32.6 megabytes.

### 6.2 Object database size

`stress-test objects`: one client, 5 seconds of ReadProperty of random
Analog Inputs, then ReadPropertyMultiple requests of 10 random objects (one
per 10 objects in the database, at most 100), at each database size.

| Objects | ReadProperty requests per second | ReadProperty median latency (microseconds) | ReadProperty 99th percentile latency (microseconds) | ReadPropertyMultiple of 10, median latency (microseconds) | Resident memory after the step (megabytes) |
| --- | --- | --- | --- | --- | --- |
| 100 | 28,078 | 34 | 52 | 41 | 13.8 |
| 500 | 28,625 | 33 | 47 | 37 | 14.8 |
| 1,000 | 28,603 | 33 | 48 | 41 | 15.5 |
| 2,500 | 28,462 | 34 | 49 | 41 | 17.1 |
| 5,000 | 28,584 | 33 | 48 | 39 | 21.2 |
| 10,000 | 28,460 | 33 | 48 | 37 | 26.2 |

Lookups are indexed, so latency does not change with the database size.
From 100 to 10,000 objects resident memory grew by 12.4 megabytes, about
1.25 kilobytes per Analog Input including the allocator's overhead. 10,000
objects is the server's default object limit.

### 6.3 COV subscriptions

The table below is from the scenario's earlier form, in which one
subscriber client subscribed to N Analog Outputs (unconfirmed
notifications, 600-second lifetime) and a second client wrote their
Present_Values in turn at 10 writes per second for 10 seconds.

`stress-test cov` now spreads each step's N subscriptions over as many
subscriber clients as the per-subscriber limit of 64 needs, each taking
Analog Outputs 1 to 64 (or fewer), so 1,024 subscriptions come from 16
subscribers and each write notifies all 16. `--confirmed` asks for
confirmed notifications, which each subscriber acknowledges. Its default
steps are 1, 10, 64, 256 and 1,024 subscriptions; re-run this section with
them, with and without `--confirmed`, at the next refresh. A short trial
run (2 seconds a step, on a loaded machine) had the server accept all
1,024 subscriptions and deliver every expected notification, confirmed or
not, and refuse the 64 subscriptions over 1,024 in a step of 1,088.

| Subscriptions requested | Subscriptions accepted | Writes | Notifications received | Notifications expected |
| --- | --- | --- | --- | --- |
| 1 | 1 | 99 | 100 | 100 |
| 10 | 10 | 98 | 108 | 108 |
| 50 | 50 | 98 | 148 | 148 |
| 100 | 64 | 98 | 128 | 128 |
| 200 | 64 | 98 | 128 | 128 |
| 255 | 64 | 98 | 128 | 128 |

Expected notifications are one initial notification per accepted
subscription plus one per write to a subscribed object. Every expected
notification arrived.

From 100 subscriptions up, the server accepted 64 and refused the rest with
the error `RESOURCES` / `NO_SPACE_TO_ADD_LIST_ELEMENT`: by default the server
admits at most 64 subscriptions from one subscriber
(`CovPolicy::max_subscriptions_per_peer`), and 1,024 in all. With one
subscriber the table measures the per-subscriber limit; the scenario's
several subscribers now reach the total.

### 6.4 Segmentation

`stress-test segmentation`: ReadPropertyMultiple of N Present_Values, 50
requests at each size. The server and its Device support segmentation in
both directions (this edition fixes the scenario, which before never
enabled it, so a reply too large for one APDU was refused).

| Objects per request | Reply size and segments | Median latency (microseconds) | 99th percentile latency (microseconds) | Failed requests |
| --- | --- | --- | --- | --- |
| 10 | about 160 bytes, one APDU | 51 | 2,447 | 0 |
| 50 | about 800 bytes, one APDU | 65 | 1,354 | 0 |
| 100 | about 1.6 kilobytes, 2 segments | 131 | 1,424 | 0 |
| 250 | about 4 kilobytes, 3 segments | 281 | 2,121 | 0 |

Latency grows with the reply's size and its segments: 250 objects in 3
segments take about four times as long as 50 in one APDU. With only 50
requests per size, the 99th percentile is the slowest one or two requests
(typically the first), not a steady-state figure.

A ReadPropertyMultiple of 500 objects was refused with an Abort (out of
resources): by default a server answers at most 256 results or 16,384
bytes in one ReadPropertyMultiple (`ReadPropertyMultipleBudget`).

### 6.5 Mixed workload

`stress-test mixed`: 10 clients for 30 seconds, each request picked at
random: 60% ReadProperty, 15% WriteProperty, 10% ReadPropertyMultiple of 5
objects, 5% a COV subscribe and unsubscribe, 5% Who-Is, and 5%
ReadProperty of the Device. The server holds 550 objects.

| Clients | Operations per second | Median latency (microseconds) | 99th percentile latency (microseconds) | Failed operations | Peak resident memory (megabytes) |
| --- | --- | --- | --- | --- | --- |
| 10 | 76,769 | 130 | 240 | 0 | 18.0 |

### 6.6 BBMD foreign devices

The table below is from the scenario's earlier form: a BBMD, which relays
broadcasts for B/IP devices, with N foreign devices registered in its
Foreign Device Table (FDT), all on loopback. The foreign devices took
turns sending a broadcast Who-Is through the BBMD for 10 seconds, with a
100-microsecond pause between sends, and the next device waited up to 10
milliseconds for the forwarded copy.

| Foreign devices requested | Registered in the FDT | Broadcasts sent per second | Median send time (microseconds) | Packets forwarded | Packets held back by the fan-out limit | Failed sends |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 1 | 741 | 13 | 0 | 0 | 0 |
| 5 | 5 | 533 | 14 | 5,120 | 16,208 | 0 |
| 10 | 10 | 532 | 36 | 5,120 | 42,805 | 0 |
| 25 | 16 | 174 | 38 | 5,120 | 13,376 | 270 |

The scenario's own pacing sets its send rate, so the rate is not a
capacity figure. What it shows is that the BBMD forwards broadcasts to
every registered foreign device and that its fan-out limits engage: the
scenario configures at most 512 forwarded packets per second from one
origin, so the BBMD holds the rest back rather than flooding the network.

Only 16 of 25 foreign devices registered: the BBMD admits at most 16
management requests per source address per second, a fixed guard against
floods, and all 25 devices share the address 127.0.0.1 and register within
the same second. On a real network each foreign device has its own
address.

`stress-test bbmd` now measures capacity. Every foreign device sends
broadcasts through the BBMD as fast as its socket takes them, with no
pause and no waiting, for the step's duration, and the BBMD's fan-out
limits are set far above what loopback carries. The step's rate is the
packets the BBMD forwarded per second; it also reports the broadcasts
offered, the copies delivered to the foreign devices, and the forwarding
jobs dropped when the BBMD's queue was full, which is where its capacity
runs out. On Linux each foreign device binds its own loopback address
(127.1.0.1, 127.1.0.2 and so on), so the registration guard no longer caps
the count; on macOS they still share 127.0.0.1. Run it in the Docker
orchestrator container ([section 7.3](#73-reproducing-the-docker-run)) for
the published figure, and replace this table at the next refresh. A short
trial run on macOS (2 seconds a step, on a loaded machine) forwarded about
30,000 packets per second with 5 foreign devices; in the Linux container
all 100 of 100 foreign devices registered. The 1-device step always
forwards nothing: a BBMD doesn't send a broadcast back to the foreign
device it came from, and there is no other.

### 6.7 Device scan

`stress-test device-scan` (named `whois` before
[#1714](https://github.com/jscott3201/rusty-bacnet/issues/1714)): N
servers on loopback, each its own device; one client reads every Device's
Object_Name in turn, as a supervisory tool does once it knows the devices.
It does not broadcast a Who-Is: on loopback each server has its own UDP
port, so a broadcast would reach only one of them. The Docker
orchestrator's discovery scenario
([section 7.1](#71-cross-network-orchestrator)) measures Who-Is discovery.

| Devices | Devices read | Time to read all (milliseconds) | Median read time (microseconds) | 99th percentile read time (microseconds) |
| --- | --- | --- | --- | --- |
| 5 | 5 | 2 | 125 | 1,484 |
| 10 | 10 | under 1 | 84 | 209 |
| 25 | 25 | 2 | 113 | 184 |
| 50 | 50 | 6 | 116 | 1,312 |

Every device answered. The first request to each new server is slower than
the steady-state requests of [section 6.1](#61-concurrent-clients), and the
99th percentile is the slowest one or two of them. The scenario's rate,
devices read per second, now divides by the scan's measured time at full
resolution; it used to divide by whole milliseconds, which meant nothing
for a scan of under a millisecond. It has no duration option: each step
is one pass over the devices.

## 7. Docker cross-network results

`examples/docker` runs the stack as separate Linux containers on Docker
bridge networks, so packets cross real (virtual) network interfaces instead
of loopback. Docker Desktop runs the containers in a Linux virtual machine
with 4 processors on the same laptop. The images are Alpine Linux with
statically linked release binaries. The BACnet/SC pair needs credentials:
this run used the development-only recipe in `examples/docker/README.md`,
with the files kept outside the checkout and deleted afterwards.

The topology: B/IP network 1 (172.20.0.0/24) with `server-a` (1,000
objects) and `bbmd-a`; B/IP network 2 (172.20.1.0/24) with `server-b`
(1,000 objects) and `bbmd-b`; the `router` service (`BACnetRouter`) joining
networks 1 and 2; a BACnet/SC hub and device; a foreign-device network
(172.20.4.0/24), which the `ip-router` service joins to network 1 as a
site's IP router would; and a discovery network (172.20.5.0/24) holding
the `fleet` service, 20 copies of one device by default, each with its own
address and Device instance. The orchestrator container sits on networks
1 and 2, the BACnet/SC network and the discovery network, and drives the
tests; the `foreign-bench` container sits on the foreign network alone and
runs the foreign-device scenario. It is in the compose file's `foreign`
profile, so a plain `docker compose up` does not run it alongside the
orchestrator. Docker does not route IP between its
bridge networks, so the hosts on the foreign device's path add routes
through `ip-router` when they start, which needs the `NET_ADMIN`
capability.

### 7.1 Cross-network orchestrator

`stress-orchestrator --duration 10`: one client doing ReadProperty requests
one after another for 10 seconds. Its scenarios are `same-subnet`,
`router`, `discovery` and `foreign`; `--only` and `--skip` pick them, and
without `--only` every one but `foreign` runs. The summary covers every
scenario it ran; it used to report the requests as each path's rate times
the duration, and none as failed.

| Path | Requests per second | Median latency (microseconds) | 99th percentile latency (microseconds) | Failed requests |
| --- | --- | --- | --- | --- |
| Same network: orchestrator to `server-a` | 27,944 | 30 | 89 | 0 |
| Routed: orchestrator on network 1, through `router`, to `server-b` on network 2 | 20,916 | 43 | 120 | 0 |

The router hop added 13 microseconds to the median, close to the 18
measured on macOS loopback in [section 5](#5-routing). An earlier run of
the same-network path, before the fix below, gave 34,109 requests per
second at a median of 27 microseconds: the virtual machine's numbers vary
by about 20% between runs.

Two fixes were needed for this run, both in the benchmark harness, not the
stack:

- The orchestrator's "router hop" sent its requests straight to
  `server-b`'s IP address and relied on IP routing between Docker bridge
  networks, which Docker blocks, so every request timed out and no BACnet
  routing was involved. It now sends them to the `router` service with
  the destination network and address in the NPDU.
- The `bacnet-router` binary bound each port's B/IP transport to the
  wildcard address. Both ports use UDP port 47808, so on Linux one socket
  received every port's unicast and the router could not tell the networks
  apart. It now binds each port to its own address
  (`BipTransport::set_share_port_by_address`).

This run had no foreign-device or discovery figure. The foreign-device
scenario then never registered with a BBMD and, like the old router hop,
depended on IP routing between bridge networks, so every request timed
out. Since [#1714](https://github.com/jscott3201/rusty-bacnet/issues/1714)
two scenarios cover them; measure both at the next refresh:

- **`foreign`**, run by the `foreign-bench` service from the foreign
  network: a client registers as a foreign device with `bbmd-a`, finds
  `server-a` with a Who-Is that `bbmd-a` distributes on network 1, then
  reads from `server-a` for the duration. Only the Who-Is goes through
  the BBMD: by default a server sends its I-Am straight back to the
  device that asked (`DiscoveryPolicy::prefer_directed_responses`), so
  the I-Am and the reads are unicast over `ip-router`. It reports the
  time to register and to discover, and the reads' rate and latency.
- **`discovery`**: the orchestrator broadcasts a Who-Is on the discovery
  network and times how long until every device of the fleet has
  answered with an I-Am, round after round for the duration. A round in
  which some device does not answer within 2 seconds counts as failed.
  `--fleet` tells it how many devices to expect (the fleet's replicas).
  Rounds start 250 milliseconds apart, off the clock: by default a server
  ignores a repeat of the same Who-Is from one source within 200
  milliseconds (`DiscoveryPolicy::coalesce_window`).

A short trial of both (2 seconds each, on a heavily loaded machine) found
`server-a` through the BBMD 5 milliseconds after registering, and read
from it with no failures; and all 20 devices of the fleet answered in
every one of 9 discovery rounds.

### 7.2 Stress scenarios inside a container

The same `stress-test` scenarios as [section 6](#6-scale-and-stress), run
inside one orchestrator container, over the container's loopback interface.
This compares the Linux network stack with macOS's on the same hardware.

| Scenario | Linux container: requests per second | Linux container: median latency (microseconds) | macOS host: requests per second | macOS host: median latency (microseconds) |
| --- | --- | --- | --- | --- |
| 1 client | 45,373 | 21 | 30,502 | 32 |
| 5 clients | 117,006 | 40 | 70,884 | 68 |
| 10 clients | 88,937 | 102 | 73,750 | 132 |
| 25 clients | 104,914 | 230 | 69,895 | 353 |
| 50 clients | 110,152 | 416 | 65,016 | 749 |
| Mixed workload, 10 clients (10 seconds in the container, 30 on the host) | 99,202 | 99 | 76,769 | 130 |
| Routed B/IP to B/IP, 1 client | 38,526 | 24 | 19,366 | 49 |
| Routed B/IP to B/IP, 25 clients | 81,119 | 302 | 55,874 | 435 |

| MS/TP small reply, baud rate | Linux container: requests per second | Linux container: share of the wire limit | macOS host: requests per second | macOS host: share of the wire limit |
| --- | --- | --- | --- | --- |
| 9,600 | 10.2 | 89% | 10.4 | 91% |
| 38,400 | 32.4 | 71% | 34.4 | 75% |
| 76,800 | 47.1 | 51% | 53.2 | 58% |
| 115,200 | 63.8 | 47% | 72.7 | 53% |

The Linux container answered 20% to 70% more requests than macOS on the
same machine with 4 of its 10 processors (about 50% in most rows), and the
routed path gained most, up to twice the rate:
the per-datagram cost on macOS is higher than on Linux. A Linux server is
the better guide to production. MS/TP, which is bound by timers rather than
datagrams, ran a little slower in the virtual machine. Peak resident memory
in the container was 9 to 10 megabytes per scenario.

### 7.3 Reproducing the Docker run

Provision the BACnet/SC credentials first, as `examples/docker/README.md`
describes, then, from `examples/docker`:

```bash
export SC_CERT_DIR=/absolute/path/outside/the/checkout
export COMPOSE_PROJECT_NAME=rusty-bench
# --profile foreign includes foreign-bench, which a plain build or up leaves out.
docker compose --profile foreign build
# Everything but the two runners, whose own commands would run alongside yours.
docker compose up -d server-a bbmd-a server-b bbmd-b router sc-hub sc-server \
  ip-router fleet
docker compose run --rm -T --no-deps orchestrator \
  stress-orchestrator --wait-for-ready=20 --duration 10 --fleet 20
docker compose run --rm -T --no-deps foreign-bench \
  stress-orchestrator --wait-for-ready=5 --duration 10 --only foreign
docker compose run --rm -T --no-deps orchestrator \
  stress-test clients --steps 1,5,10,25,50 --duration 10
# BBMD capacity, one loopback address per foreign device.
docker compose run --rm -T --no-deps orchestrator \
  stress-test bbmd --steps 1,5,10,25,50,100 --duration 10
docker compose down -v --rmi local
```

For a larger fleet, start it with `--scale fleet=50` and pass the
orchestrator `--fleet 50`. `stress-test clients --transport bip6` needs an
IPv6 loopback address, which Docker containers lack unless IPv6 is enabled
for them, so run it on the host. Without BACnet/SC credentials, leave `sc-hub`
and `sc-server` out of the list.

## 8. Embedded footprint

The portable crates build for microcontrollers with no operating system.
`firmware/link` links one small image per stack for four targets, with the
size-optimized `firmware` profile, and `scripts/ci/check-firmware-link.sh`
checks each against `firmware/link/size-budget.toml`. These sizes were
measured for this report with that script (raw output:
`firmware-link.txt`); all images were within their budgets.

The images:

- `mstp`: an MS/TP master node (the frame codec and the Clause 9 master
  core) with standard frames.
- `mstp-extended`: the same with extended frames, for NPDUs up to 1,497
  bytes.
- `bip`: the B/IP and B/IPv6 BVLL codecs.
- `node`: a B/IP foreign-device node.
- `bbmd`: a BBMD's broadcast and foreign device tables.
- `router`: a two-port router, a BBMD's B/IP port to an MS/TP port with
  extended frames.
- `apdu`: the application layer encoder and decoders, with a 1-kilobyte
  heap; the only image with an allocator.
- `device`: a whole MS/TP device with no heap: the data link, the
  network layer and an application layer that answers Who-Is and
  ReadProperty. It joined after the tables below were measured; its sizes
  follow them.

Flash is code, read-only data and the vector table. RAM is static RAM
(initialized and zeroed data, the heap arena included), not the stack.
Sizes in bytes.

| Image | Cortex-M0+ (`thumbv6m-none-eabi`) flash | Cortex-M3 (`thumbv7m-none-eabi`) flash | Cortex-M4F (`thumbv7em-none-eabihf`) flash | RISC-V (`riscv32imc-unknown-none-elf`) flash | Static RAM, all targets |
| --- | --- | --- | --- | --- | --- |
| `mstp` | 18,840 | 18,440 | 18,328 | 20,118 | 5,528 |
| `mstp-extended` | 17,584 | 17,128 | 17,112 | 18,862 | 7,696 |
| `bip` | 11,640 | 10,936 | 10,868 | 13,366 | 132 |
| `node` | 14,252 | 13,496 | 13,392 | 14,570 | 64 |
| `bbmd` | 10,648 | 9,800 | 9,752 | 11,202 | 240 |
| `router` | 47,600 | 45,088 | 44,888 | 51,286 | 15,504 |
| `apdu` | 45,176 | 43,616 | 43,528 | 51,182 | 1,056 |

The deepest stack each image reached, measured under QEMU emulation by the
manual check `scripts/emu/emulate-firmware.sh` (Rust 1.99.0, QEMU 10.0.13;
not re-run for this report; see `docs/ci.md`, "Emulation (manual)", and
`firmware/link/stack-budget.toml`). Sizes in bytes.

| Board (target) | `mstp` | `mstp-extended` | `bip` | `node` | `bbmd` | `router` | `apdu` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| micro:bit (`thumbv6m-none-eabi`) | 2,040 | 3,020 | 2,220 | 4,716 | 2,300 | does not fit | 2,768 |
| LM3S6965 (`thumbv7m-none-eabi`) | 1,824 | 2,868 | 2,100 | 4,188 | 2,228 | 5,648 | 2,536 |
| Netduino Plus 2 (`thumbv7em-none-eabihf`) | 1,832 | 2,868 | 2,100 | 4,188 | 2,228 | 5,640 | 2,536 |
| SiFive E (`riscv32imc-unknown-none-elf`) | 1,712 | 2,732 | 2,020 | 4,008 | 2,132 | does not fit | 2,464 |

The `device` image, measured when it joined (#1771) with the same
toolchain and QEMU: 29,876 bytes of flash on Cortex-M0+, 28,776 on
Cortex-M4F, and 3,496 bytes of static RAM on every target; its deepest
stack was 2,652 bytes on the micro:bit board and 2,284 on the LM3S6965.

What an embedded project can expect:

- An MS/TP device needs about 19 kilobytes of flash and 7.6 kilobytes of
  RAM (5.5 static, 2 of stack) for the data link; with extended frames,
  about 10.7 kilobytes of RAM.
- The application layer image (`apdu`) is about 44 kilobytes of flash on
  its own, plus its heap.
- A whole MS/TP device that answers Who-Is and ReadProperty without a
  heap (`device`) needs about 30 kilobytes of flash and 6.2 kilobytes of
  RAM (3.5 static, 2.7 of stack). Without the heap, the application layer
  links no formatting code, and the device is smaller in flash than the
  `apdu` image alone.
- The router needs about 21 kilobytes of RAM (15.5 static, 5.6 of stack),
  so it does not fit the 16-kilobyte boards; most of its static RAM is the
  extended-frame buffers an MS/TP router port requires.
- Each image links its own test scenario as well as the stack, so compare
  images with care: `mstp-extended` is smaller in flash than `mstp`
  because their scenarios differ.

## 9. Python bindings

The `rusty-bacnet` package wraps the same Rust client and server for
Python's asyncio. `benchmarks/python` holds four pytest suites, run here on
CPython 3.13.15 against a release build of the package (built with
`maturin develop --release`), over loopback. Each measures 100 or 200
requests, one after another, after a warm-up; the concurrency tests run
50 requests per concurrent task, with that many in flight at once from one
client. These are single runs of small samples.

The tables below come from the suites' earlier layout, whose file names
dated from before all four started both the client and the server
through the Python package, in one process; the three B/IP suites
measured the same path three times, and agreed within 10%, which shows
the run-to-run spread. Since
[#1714](https://github.com/jscott3201/rusty-bacnet/issues/1714) one suite,
`bench_bip.py`, replaces those three, and `bench_sc.py` stays. Their
concurrency tests keep each client at or under the server's limit of 16
confirmed requests in flight from one peer: `bench_bip.py` runs 1, 5, 10,
16, 32 and 64 requests in flight, spread over up to 4 clients for the
last two, and `bench_sc.py` runs 1, 5, 10 and 16 from one client. Re-run
both tables with them at the next refresh.

| Suite | ReadProperty median (microseconds) | WriteProperty median (microseconds) | ReadPropertyMultiple of 3 objects and 2 properties, median (microseconds) | COV subscribe and cancel, median (microseconds) |
| --- | --- | --- | --- | --- |
| `bench_py_client_rust_server.py` (B/IP) | 102 | 72 | 73 | 159 |
| `bench_rust_client_py_server.py` (B/IP) | 95 | 72 | 73 | 157 |
| `bench_py_py.py` (B/IP) | 96 | 70 | 73 | 159 |
| `bench_sc.py` (BACnet/SC with mutual TLS, through a hub) | 173 | 106 | 97 | 189 |

| Requests in flight from one client | `bench_py_client_rust_server.py`: requests per second | `bench_rust_client_py_server.py`: requests per second | `bench_py_py.py`: requests per second | `bench_sc.py`: requests per second |
| --- | --- | --- | --- | --- |
| 1 | 5,296 | 5,540 | 5,644 | 8,560 |
| 5 | 14,918 | 12,499 | 12,986 | 26,711 |
| 10 | 23,317 | 23,975 | 24,478 | 20,120 after the fix (50 before it, stalled; see below) |
| 25 | failed (see below) | 33,963 | failed (see below) | 62 (stalled) |

What it shows:

- A request from Python costs 70 to 100 microseconds, against 35 from Rust
  ([section 4](#4-transports)): the asyncio event loop and the crossing
  between Python and Rust add 35 to 65 microseconds per call.
- Python reaches 23,000 to 24,000 requests per second with 10 requests in
  flight, and 34,000 with 25 where that ran, about half of what Rust
  clients reach on the same machine.
- With 25 requests in flight from one client, some were aborted (out of
  resources) in two of the three B/IP suites: by default a server admits
  at most 16 confirmed requests in flight from one peer. Spread the
  requests over several clients, or raise the server's limit, to go
  further.
- **BACnet/SC stalled with 10 or more requests in flight from one Python
  client, until a fix:** most requests still took about 0.15 milliseconds,
  but a few waited for the 5-second timeout, so the rate fell to 50 to 62
  requests per second. The cause was the admission quota on a BACnet/SC
  node's receive queue: it held at most 4 messages from any one peer and
  dropped the rest of a burst without telling the sender, whose request
  then timed out. [#1712](https://github.com/jscott3201/rusty-bacnet/issues/1712)
  raised the quota to 32 messages, in a queue of 256. Measured again on
  13 October 2026 after the fix, 10 requests in flight ran at 20,120
  requests per second, with the slowest request under 2 milliseconds; four
  runs ranged from 15,000 to 32,000 requests per second. With 25 in flight
  the server now refuses the requests over its limit of 16 per peer
  (aborted, out of resources), as on B/IP; the 25 row predates the fix.
- The suites' Who-Is test waits a fixed 300 milliseconds for answers, so
  its 302-millisecond time measures the wait, not the stack; the test is
  now named `test_bip_whois_fixed_wait` to say so.

To run them:

```bash
uv venv .venv --python 3.13 && source .venv/bin/activate
uv pip install pytest pytest-benchmark pytest-asyncio psutil maturin
maturin develop --release -m crates/rusty-bacnet/Cargo.toml
cd benchmarks/python
python -m pytest bench_bip.py -s -v   # B/IP, client and server from Python
python -m pytest bench_sc.py -s -v    # BACnet/SC through a hub
```

## 10. Guidance for future work

Where the time goes, and what to measure next.

**Bottlenecks seen in this run.**

- **The IP round trip is the stack, not the encoding.** Encoding and
  decoding a request take under 0.1 microseconds of a 31- to 35-microsecond
  round trip. Most of the rest is likely task hand-offs between the
  socket, network and application layers and the operating system's UDP
  path, twice per request: a profile of one round trip is where to start.
- **macOS's datagram path is slow; Linux is the better guide.** The same
  binaries in a Linux container on 4 of the laptop's processors answered
  20% to 70% more requests than on macOS, and up to twice as many routed
  ([section 7.2](#72-stress-scenarios-inside-a-container)). Performance
  work on the IP transports should be measured on Linux.
- **The IP round trip is slower than in the 0.8.0 report.** The previous
  edition measured a B/IP ReadProperty at 20.9 microseconds and one stress
  client at 73,948 requests per second, on a machine of the same model.
  This run measured 35.5 microseconds and 30,502. The operating system and
  compiler differ between the two runs, so this is not yet a confirmed
  regression. An A/B run of 0.8.0 against 0.13.0 on one machine
  (`scripts/bench-ab.sh`) would settle it, and decode of a ReadProperty
  request (4.3 nanoseconds then, 19.9 now) is a small place to start
  ([#1713](https://github.com/jscott3201/rusty-bacnet/issues/1713)).
- **BACnet/SC stalled with several requests in flight; now fixed.** From
  Python, 10 or more concurrent requests from one client over BACnet/SC
  left a few requests waiting for the 5-second timeout
  ([section 9](#9-python-bindings)); B/IP did not. The cause was the
  admission quota on a BACnet/SC node's receive queue, 4 messages from any
  one peer, which dropped the rest of a burst without telling the sender.
  [#1712](https://github.com/jscott3201/rusty-bacnet/issues/1712) raised it
  to 32, and a dropped message now logs a warning, at most one every 10
  seconds, with the count since the last one.
- **Concurrency peaks early.** One server peaks at 5 to 10 clients, near
  74,000 requests per second, and loses a little beyond that. Profiling the
  server's dispatch under 25 to 100 clients would show whether the limit is
  a lock, the request admission control, or the single UDP socket.
- **MS/TP's host overhead is per frame.** About 1.5 to 2 milliseconds per
  frame of timer and scheduling delay halves small-request throughput at
  115,200 baud. The MS/TP transport's `DedicatedThread` execution mode,
  or finer timers, would be the next thing to measure (see also
  [#501](https://github.com/jscott3201/rusty-bacnet/issues/501), a
  deterministic Linux execution mode).
- **The router hop costs 13 to 18 microseconds**, about half of a direct
  request. Most of it is two more datagrams; a profile of the router's
  per-packet path would show what is left to cut.

**What to measure next.**

- The MS/TP scenario on real RS-485 hardware with a USB adapter, to compare
  with the simulated line.
- Stack peaks re-measured under emulation after the next toolchain bump.

**Harness changes awaiting a re-run.** The harness follow-ups of
[#1714](https://github.com/jscott3201/rusty-bacnet/issues/1714) changed
what these measure, so the next refresh should re-run them and replace
their figures:

- [Section 6.1](#61-concurrent-clients): every client now connects before
  the clock starts; add rows for `--transport bip6` and `--transport sc`.
- [Section 6.3](#63-cov-subscriptions): several subscribers up to the
  server's total of 1,024, with and without `--confirmed`.
- [Section 6.6](#66-bbmd-foreign-devices): capacity without pacing, run in
  the Docker orchestrator container for one address per foreign device.
- [Section 6.7](#67-device-scan): the device scan's rate at full
  resolution (the per-device latencies are unchanged).
- [Section 7.1](#71-cross-network-orchestrator): the new `foreign` and
  `discovery` scenarios.
- [Section 9](#9-python-bindings): `bench_bip.py` in place of the three
  B/IP suites, and both suites' concurrency within 16 requests per client.
- Any figure taken from a scenario's summary rather than its steps: the
  summary now covers every step.

**Still to fix in the harness.** `stress-test mixed` and `stress-test
router` still create their clients inside the timed window, record into
one histogram behind a shared lock, and divide by the nominal duration
rather than the measured one. `stress-test clients` no longer does any of
these; the other two should follow it in a later change.

## 11. How to reproduce

Run one suite at a time on an idle machine. Each Criterion suite takes 1 to
2 minutes, each stress scenario 10 seconds per step.

```bash
# Criterion suites
cargo bench -p bacnet-benchmarks --bench encoding
cargo bench -p bacnet-benchmarks --bench codecs
cargo bench -p bacnet-benchmarks --bench read_range
cargo bench -p bacnet-benchmarks --bench bip_latency
cargo bench -p bacnet-benchmarks --bench bip_throughput
cargo bench -p bacnet-benchmarks --bench bip6_latency
cargo bench -p bacnet-benchmarks --bench bip6_throughput
cargo bench -p bacnet-benchmarks --bench sc_mtls_latency
cargo bench -p bacnet-benchmarks --bench sc_mtls_throughput

# Stress scenarios (build once, then run the binary)
cargo build --release -p bacnet-benchmarks --bins
ST=./target/release/stress-test
$ST clients --steps 1,5,10,25,50,100 --duration 10
$ST clients --transport bip6 --steps 1,5,10,25,50,100 --duration 10
$ST clients --transport sc --steps 1,5,10,25,50,100 --duration 10
$ST objects --steps 100,500,1000,2500,5000,10000 --duration 5
$ST cov --steps 1,10,64,256,1024 --duration 10
$ST cov --steps 1,10,64,256,1024 --duration 10 --confirmed
$ST segmentation --steps 10,50,100,250
$ST mixed --clients 10 --duration 30
$ST router --path direct --steps 1,5,10,25,50 --duration 10
$ST router --path bip --steps 1,5,10,25,50 --duration 10
$ST router --path mstp --baud 115200 --steps 1,5 --duration 10
$ST mstp --reply small --duration 10
$ST mstp --reply standard --duration 10
$ST mstp --reply extended --duration 20
$ST bbmd --steps 1,5,10,25,50,100 --duration 10   # on Linux; see section 7.3
$ST device-scan --steps 5,10,25,50

# Embedded image sizes (needs the four targets: rustup target add ...)
bash scripts/ci/check-firmware-link.sh
```

The Docker and Python steps are in their own sections above.
`scripts/bench-local.sh` and `scripts/bench-ab.sh` wrap these suites and
compare two revisions; their output goes to `bench-output/`, which git
ignores. Results promoted into a tracked history should follow
`benchmarks/schema/benchmark-result.schema.json`.
