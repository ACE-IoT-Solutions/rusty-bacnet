# Segmented reads and confirmed COV with BACnet4J

This opt-in Linux B/IP fixture runs a Java device and a Rusty Python client in
separate processes. BACnet4J's regular Device object handles RP and RPM. Its
transport proposes a window of two, chooses the segment size and sends the
datagrams. The Java control commands set up discovery and call the normal
confirmed notification service.

The fixture uses Maven artifact `com.infiniteautomation:bacnet4j:6.1.1`.
The POM identifies GPL-3.0. The JAR is a test-only dependency in the peer
image. The image build verifies the JAR's SHA-256,
`588dc6e986dd118fee24d436420cbbb745aa1a7d9e8583e15e2d93deeb49e97f`.
The separately inspected source JAR's SHA-256 was
`57af8a82dba73db6acc443a135e98f25befb3a3a6561a1780dc1e369500ac8aa`.
The executed classpath is identified by artifact hashes. [Maven POM](https://maven.mangoautomation.net/repository/ias-release/com/infiniteautomation/bacnet4j/6.1.1/bacnet4j-6.1.1.pom)
and [corresponding Maven sources](https://maven.mangoautomation.net/repository/ias-release/com/infiniteautomation/bacnet4j/6.1.1/bacnet4j-6.1.1-sources.jar).

## Reproduce

Use Podman and a local Python client image with CPython 3.13, `venv` and
the native wheel's runtime libraries for the target architecture. Set
`RUSTY_QA_IMAGE` to its image ID or name. The Java image build needs the
Maven repositories. Both protocol processes run offline. The peer starts in
a new network namespace with `--network none`, and the client joins that
namespace. Both use IPv4 loopback. No ports are published on the host.

```sh
fixture=scripts/interop/bacnet4j
podman build -t localhost/rusty-bacnet-interop-bacnet4j:6.1.1 "$fixture"
mkdir -p target/bacnet4j
"$fixture/run.sh" "$RUSTY_WHEEL" "$RUSTY_SOURCE_SHA" target/bacnet4j/candidate.json
```

The wheel and output directory are mounted from the host. The runner installs
only the supplied wheel in a fresh client environment. The receipt verifies
its native extension against the installed binary and records both SHA-256
digests, the declared Rusty source SHA, container image IDs, Java runtime,
JAR and class hashes, fixture source hashes and Java command replies. It also
records each original UDP payload with direction and a monotonic timestamp.
A failed SDK call or trace assertion produces a nonzero exit. Append
`--reads-only` when checking a wheel that predates exact COV row octets.

The relay forwards the original UDP bytes in each direction. For every
segmented response or confirmed COV the trace checks the invoke, service,
window, sequence and ACK positions. In particular, it observes the first
positive ACK before segment 1 and the last SegmentACK before a COV terminal
result. Both RP and the two requested RPM occurrences must return exactly
6,000 characters. The shared endpoint uses its public 480-byte receive
budget; the standalone client uses its normal 1,476-byte budget.

## Observed Linux arm64 results

These runs used CPython 3.13, Java `17.0.20.1+1` and BACnet4J 6.1.1.
The candidate wheel from `97015c3d3` had SHA-256
`63958ccb72b6ba44f76bb477ac57b6b98501354d36d7742f38f4503b4efb8265`.
The separate reads-only control used the published 0.17.2 manylinux aarch64
wheel from `aa5c45236`, SHA-256
`4a5b795ae65184b1088c6dc42f4df58892d5f1a80af4b2f4b3589bb4d41c4f73`.
Both wheels gave the following results for the same Java Device Description.

| Rusty owner | Observed maximum segment APDU | RP segments / ACKs | RPM segments / ACKs |
| --- | ---: | ---: | ---: |
| Standalone | 1,476 bytes | 5 / 3 | 9 / 5 |
| Shared B/IP endpoint | 480 bytes | 13 / 7 | 26 / 14 |

For the candidate's shared endpoint, BACnet4J learned Device 100's
480-byte budget and receive capability from its answer to a directed Who-Is.
BACnet4J sent a 6,000-character confirmed notification for an unknown process
and then a distinct notification for the registered process. Each used 13
segments and seven positive ACKs, with a maximum APDU of 480 bytes. Java
received Services / Unknown_Subscription for the first and SimpleACK for the
second. The next COV event identified the admitted process, source and Device
object, contained its full Description and preserved the original tagged value
from the capture. The COV iterator reported zero missed events.

This qualifies an independent software stack on isolated Linux loopback.
Physical controllers, deployed network interfaces, routing and vendor-specific
object behavior require field evidence.
