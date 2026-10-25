---
title: "Python reference"
description: "Every class, method, signature, keyword default and docstring of the rusty_bacnet module, generated from its type stub."
---

This page lists the `rusty_bacnet` module as its type stub, `rusty_bacnet.pyi`, describes it: classes, methods, signatures with their keyword defaults, and docstrings. The stub ships in the wheel, so an editor shows the same signatures. Async methods return native awaitables; await them on a running asyncio loop.

Signatures and short contracts are here. Lifecycle, cancellation and error contracts that are too long for a docstring are in the [Python API guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/python-api.md). For tasks, start with [Read with Python](/rusty-bacnet/start/python/) or the [local lab](/rusty-bacnet/start/local-lab/).

`website/scripts/reference-python.py` writes everything below from the stub; edit the stub, not this part of the page.

<!-- generated:start by website/scripts/reference-python.py; do not edit below this line -->

Each group opens with its main classes, then lists the other classes, the dictionaries (`TypedDict`) and the type aliases, in stub order.

## Client

The client, its options and what its methods return.

### BACnetClient

Async BACnet client for reading/writing properties on remote devices.

Supports BACnet/IP (`"bip"`), BACnet/IPv6 (`"ipv6"`),
BACnet/SC (`"sc"`), and BACnet MS/TP (`"mstp"`) transports.

For SC, sc\_ca\_cert, sc\_client\_cert, and sc\_client\_key must be nonempty
paths; omission, None, or empty strings raise ValueError at construction.
Their None defaults preserve non-SC use and positional layout only.
Async entry loads the explicit site CA and operational cert/key; invalid
TLS configuration raises RuntimeError before dialing. No system-root or
unauthenticated-client fallback is available.

SC also requires keyword-only sc\_device\_uuid: exactly 16 bytes, not all zero,
copied at construction. Missing, None, wrong length, or all-zero values
raise ValueError after credential-presence validation, before file/network I/O.
Provision before deployment and durably reuse the same UUID for the device's
lifetime. No generation, persistence, change detection, or version/variant
enforcement is provided. Non-SC transports ignore this option.

Keyword-only reconnect=None preserves the default without automatic
redial. Supply ScReconnect for SC retries with the same loaded TLS policy,
VMAC, UUID and heartbeat settings. Other transports reject this option.

stop() and context exit refuse new work, drain outstanding native operations
and join teardown. Cancelling the close Future leaves shutdown running;
await stop() again to join it. Drain duration depends on outstanding requests.
Entry is exclusive; entering an already running or closing client fails.

Usage:

```python
async with BACnetClient() as client:
    await client.who_is()
    devices = await client.discovered_devices()
    value = await client.read_property("192.168.1.100:47808", oid, pid)
    print(value.tag, value.value)

# MS/TP peer address is a station MAC string ("7" or "mstp:7")
async with BACnetClient(transport="mstp", serial_port="/dev/ttyUSB0", mstp_mac=3) as client:
    value = await client.read_property("7", oid, pid)

# A B/IPv6 peer is named by its VMAC: "bip6:1234" for device 1234,
# "bip6:0x40ABCD" for any VMAC. resolve_vmac finds it from "[ip]:port".
# ipv6_device_instance gives the client a stable VMAC of its own.
async with BACnetClient(transport="ipv6", port=0, ipv6_device_instance=4321) as client:
    vmac = await client.resolve_vmac("[2001:db8::10]:47808")
    value = await client.read_property(vmac, oid, pid)
```

Every `address` takes `"ip:port"` (B/IP), `"bip6:<device instance>"` or `"bip6:0x<6 hex digits>"` (a B/IPv6 VMAC, ipv6 only),
colon hex such as `"01:02:03:04:05:06"` (SC, or `"00:04:d2"` for a
VMAC), or an MS/TP station. `"[ip]:port"` raises ValueError: use
`resolve_vmac`. So does a `bip6:` address on any other transport.

#### BACnetClient.__init__

```python
def __init__(
    self,
    interface: str = "0.0.0.0",
    port: int = 0xBAC0,
    broadcast_address: str = "255.255.255.255",
    apdu_timeout_ms: int = 6000,
    transport: str = "bip",
    sc_hub: Optional[str] = None,
    sc_vmac: Optional[bytes] = None,
    sc_ca_cert: Optional[str] = None,
    sc_client_cert: Optional[str] = None,
    sc_client_key: Optional[str] = None,
    sc_heartbeat_interval_ms: Optional[int] = None,
    sc_heartbeat_timeout_ms: Optional[int] = None,
    ipv6_interface: Optional[str] = None,
    *,
    serial_port: Optional[str] = None,
    mstp_baud: int = 38400,
    mstp_mac: int = 1,
    mstp_max_master: int = 127,
    mstp_max_info_frames: int = 1,
    mstp_extended_frames: bool = False,
    mstp_echo: bool = False,
    mstp_echo_grace_ms: Optional[int] = None,
    mstp_transmit_latency_pad_ms: Optional[int] = None,
    mstp_usage_timeout_ms: Optional[int] = None,
    mstp_reply_timeout_ms: Optional[int] = None,
    sc_device_uuid: Optional[bytes | bytearray] = None,
    share_port_by_address: bool = False,
    min_request_interval_ms: int = 0,
    ipv6_device_instance: Optional[int] = None,
    bbmd: Optional[Bbmd] = None,
    foreign_device: Optional[ForeignDevice] = None,
    multicast: Optional[Multicast] = None,
    reconnect: Optional[ScReconnect] = None,
    sc_npdu_per_origin_limit: int | None = None,
    sc_failover_hub: str | None = None,
    sc_connect_timeout_ms: int | None = None,
    sc_control_rate_limit: tuple[int, int] | None = None,
) -> None
```

`sc_failover_hub` (SC only, needs `reconnect`, else
ValueError): a second hub's `wss://` URL, dialled with the same TLS
settings when the primary can't be reached; the client keeps trying
the primary and returns to it. `sc_connect_timeout_ms` (SC only, at
least 1, default 10000) bounds each hub dial and Connect-Request.
Follow the link with `connection_events()` and
`connection_status()` (#1829).

`bbmd` / `foreign_device` (at most one, else ValueError): run
the client as a BBMD (B/IP only), or register it with a BBMD on
another subnet as a foreign device, so its Who-Is reaches that
subnet (#938). `foreign_device` also applies to
`transport="ipv6"` with a B/IPv6 BBMD, and then needs
`ipv6_device_instance` (#1941). See `Bbmd` and
`ForeignDevice`.

`ipv6_device_instance` (B/IPv6 only, default None): the device
instance whose VMAC the client uses (Clause H.7.2), 0 to 4194303, so
peers know it by a stable VMAC such as `bip6:0x0010E1`. Give a
client an instance to get a stable VMAC; without one it picks a
random VMAC at each start, and that needs a multicast-capable link at
start, which Linux loopback is not. Another transport, or an instance
out of range, raises ValueError.

`sc_npdu_per_origin_limit` (SC only, default None): maximum
queued, unconsumed NPDUs per ingress path and source. None keeps 32;
explicit values must be 1 through 256. Invalid values or a non-SC
transport raise ValueError before file or network I/O. Negative or
unrepresentably large values raise OverflowError. The aggregate
receive queue remains 256; see `sc_npdu_drop_counts`.

`sc_control_rate_limit` (SC only, default None): `(burst, per_second)`, the network-layer messages one source may send back to
back and then each second (#1969). None keeps 256 then 32 a second.
The burst must be 1 through 4096 and the rate at least 1; invalid
values or a non-SC transport raise ValueError before file or network
I/O, and negative values raise OverflowError. A message over its
source's budget is dropped and counted in `control_rate_drops`.

`mstp_extended_frames` (MS/TP only, default False): take and send
extended frames (Types 32 and 33), which carry NPDUs of up to 1497
octets, so APDUs of up to 1476; without it MS/TP carries 480-octet
APDUs and counts an extended frame it hears as invalid.

`mstp_echo` (MS/TP only, default False): the adapter returns every
octet this station sends (a transceiver whose receiver stays on while
it transmits), so the transport drops that echo rather than take it
for line traffic. `mstp_echo_grace_ms` (default None, 40 ms; needs
`mstp_echo=True` and at least 1, else ValueError at start) is how
long after a frame its echo may still be arriving. A negative value,
or one above 4294967295, raises OverflowError when the client is
built.

`mstp_transmit_latency_pad_ms`, `mstp_usage_timeout_ms` and
`mstp_reply_timeout_ms` (MS/TP only, default None, the transport's
own): the time added to a frame's wire time on a port that can't
report its end (4 ms on Windows, where the port has none, 0 elsewhere;
at most 15), and T\_usage\_timeout (20 to 35, default 35) and
T\_reply\_timeout (255 to 300, default 300), in milliseconds. A value
outside those ranges raises ValueError at start, before the port
opens.

`share_port_by_address` (B/IP only, default False): bind the
interface address itself, so clients and devices on other addresses
of this host can share the port. Needs an explicit interface and a
nonzero port; broadcasts and unicast are still handled in the order
they arrived.

`min_request_interval_ms` (default 0, no pacing; at most
3,600,000, else ValueError) paces the confirmed requests to each
destination: each waits until that long after the latest one sent to
the same destination finished (reply, error or the caller giving up),
or after it was sent while it is still outstanding, checking again
when it wakes. Requests waiting together go one at a time, in no
promised order. Paging a log or polling
then leaves a slow device room for its other clients; requests to
different destinations don't wait on each other. Behind a router the
pause after a reply holds for requests made one after another only
(#1535).

#### BACnetClient.__aenter__

```python
def __aenter__(self) -> Awaitable[BACnetClient]
```

#### BACnetClient.__aexit__

```python
def __aexit__(
    self,
    _exc_type: Any = None,
    _exc_val: Any = None,
    _exc_tb: Any = None,
) -> Awaitable[None]
```

#### BACnetClient.read_property

```python
def read_property(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    array_index: Optional[int] = None,
) -> Awaitable[PropertyValue]
```

Read a single property from a remote device.

ACK object/property/index must match. Device/Network Port instance 4194303
requests accept a same-type concrete ACK. Malformed/mismatched ACKs raise
BacnetError; this method returns the property value, not ACK metadata,
shaped as `PropertyValue` describes for read results.

#### BACnetClient.write_property

```python
def write_property(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    value: PropertyValue,
    priority: Optional[int] = None,
    array_index: Optional[int] = None,
) -> Awaitable[None]
```

Write a single property on a remote device.

Priority is None or 1..16. Invalid u8 priorities raise ValueError
synchronously; integers outside u8 raise OverflowError.

#### BACnetClient.read_property_multiple

```python
def read_property_multiple(
    self,
    address: str,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]],
) -> Awaitable[list[ReadAccessResult]]
```

Read multiple properties from a remote device (ReadPropertyMultiple).

`specs` is a list of `(object_id, [(property_id, array_index), ...])` tuples.
Returns ordered object/result dictionaries. Empty request/property lists
are rejected; this standalone profile still permits property selectors.

#### BACnetClient.write_property_multiple

```python
def write_property_multiple(
    self,
    address: str,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]]]],
) -> Awaitable[None]
```

Write multiple properties on a remote device (WritePropertyMultiple).

`specs` is `[(object_id, [(property_id, value, priority, array_index), ...]), ...]`.
Empty request/property lists, ALL/REQUIRED/OPTIONAL targets and u8
priorities outside 1..16 raise ValueError synchronously for the whole
request. Priorities outside u8 raise OverflowError. None, index zero,
proprietary properties, NULL and empty list values remain allowed.
A device error raises BacnetProtocolError with
first\_failed\_write\_attempt set from its WritePropertyMultiple-Error.

#### BACnetClient.read_property_from_devices

```python
def read_property_from_devices(
    self,
    requests: list[tuple[int, ObjectIdentifier, PropertyIdentifier, Optional[int]]],
    max_concurrent: Optional[int] = None,
) -> Awaitable[list[_DeviceReadBatchResult]]
```

Read a property from multiple discovered devices concurrently.

`requests` is `[(device_instance, object_id, property_id, array_index), ...]`.
`max_concurrent` must be positive and fit the native usize; None uses 32.
Zero raises ValueError synchronously; out-of-range integers raise OverflowError.
Returns an asyncio Future, accepted by await or asyncio.ensure\_future,
not a coroutine for asyncio.create\_task. Results are returned in completion
order; request\_index is the zero-based
input occurrence, including identical duplicates. Errors are BacnetError
instances with their protocol attributes. Python construction failures
raise from the whole call; cancellation returns no partial list.
Returns `[{"request_index": int, "device_instance": int, "value": PropertyValue | None, "error": BacnetError | None}, ...]`.

#### BACnetClient.read_property_multiple_from_devices

```python
def read_property_multiple_from_devices(
    self,
    requests: list[tuple[int, list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]]],
    max_concurrent: Optional[int] = None,
) -> Awaitable[list[_DeviceRpmBatchResult]]
```

Read multiple properties from multiple devices concurrently (RPM batch).

`max_concurrent` must be positive and fit the native usize; None uses 32.
Zero raises ValueError synchronously; out-of-range integers raise OverflowError.
Returns an asyncio Future, accepted by await or asyncio.ensure\_future,
not a coroutine for asyncio.create\_task. Results are returned in completion
order; request\_index is the zero-based
input occurrence, including identical duplicates. Errors are BacnetError
instances with their protocol attributes. Python construction failures
raise from the whole call; cancellation returns no partial list.
Returns `[{"request_index": int, "device_instance": int, "results": list[ReadAccessResult] | None, "error": BacnetError | None}, ...]`.

#### BACnetClient.write_property_to_devices

```python
def write_property_to_devices(
    self,
    requests: list[tuple[int, ObjectIdentifier, PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]],
    max_concurrent: Optional[int] = None,
) -> Awaitable[list[_DeviceWriteBatchResult]]
```

Write a property to multiple devices concurrently.

Every priority must be None or 1..16; invalid u8 priorities raise
ValueError synchronously before any batch dispatch. Integers outside
u8 raise OverflowError.

`requests` is `[(device_instance, object_id, property_id, value, priority, array_index), ...]`.
`max_concurrent` must be positive and fit the native usize; None uses 32.
Zero raises ValueError synchronously; out-of-range integers raise OverflowError.
Returns an asyncio Future, accepted by await or asyncio.ensure\_future,
not a coroutine for asyncio.create\_task. Results are returned in completion
order; request\_index is the zero-based
input occurrence, including identical duplicates. Errors are BacnetError
instances with their protocol attributes. Python construction failures
raise from the whole call; cancellation returns no partial list.
Returns `[{"request_index": int, "device_instance": int, "error": BacnetError | None}, ...]`.

#### BACnetClient.who_is

```python
def who_is(
    self,
    low_limit: Optional[int] = None,
    high_limit: Optional[int] = None,
) -> Awaitable[None]
```

Broadcast a Who-Is request. Responses are collected asynchronously;
use `discovered_devices()` to retrieve them.

Give `low_limit` and `high_limit` together or not at all, each from
0 to 4194303: one alone, `low_limit` above `high_limit`, or a limit
past 4194303 raises `ValueError` before anything is sent.

#### BACnetClient.discover

```python
def discover(
    self,
    timeout_ms: int = 3000,
    low_limit: Optional[int] = None,
    high_limit: Optional[int] = None,
) -> Awaitable[list[DiscoveredDevice]]
```

Convenience: send WhoIs, wait `timeout_ms`, return discovered devices.

Give `low_limit` and `high_limit` together or not at all, each from
0 to 4194303: one alone, `low_limit` above `high_limit`, or a limit
past 4194303 raises `ValueError` before anything is sent.

#### BACnetClient.who_has_by_id

```python
def who_has_by_id(
    self,
    object_id: ObjectIdentifier,
    low_limit: Optional[int] = None,
    high_limit: Optional[int] = None,
) -> Awaitable[None]
```

Broadcast Who-Has by object identifier.

Give `low_limit` and `high_limit` together or not at all, each from
0 to 4194303: one alone, `low_limit` above `high_limit`, or a limit
past 4194303 raises `ValueError` before anything is sent.

#### BACnetClient.who_has_by_name

```python
def who_has_by_name(
    self,
    name: str,
    low_limit: Optional[int] = None,
    high_limit: Optional[int] = None,
) -> Awaitable[None]
```

Broadcast Who-Has by object name.

Give `low_limit` and `high_limit` together or not at all, each from
0 to 4194303: one alone, `low_limit` above `high_limit`, or a limit
past 4194303 raises `ValueError` before anything is sent.

#### BACnetClient.who_has_replies

```python
def who_has_replies(
    self,
    object: ObjectIdentifier | str,
    low_limit: Optional[int] = None,
    high_limit: Optional[int] = None,
    timeout_ms: int = 3000,
) -> Awaitable[list[IHave]]
```

Broadcast a Who-Has for `object` (an identifier, or a name) and
collect the I-Haves that answer it for `timeout_ms`: one per device
and object, in arrival order; an empty list when none answer. The
limits follow `who_has_by_id()`'s rule (#1850). It holds the
started client for the whole wait, so `stop()` waits up to
`timeout_ms` for it.

#### BACnetClient.i_have_events

```python
def i_have_events(self) -> Awaitable[IHaveIterator]
```

Follow every I-Have the client receives from now on, an answer to
its own Who-Has or not (#1850). RuntimeError before start and after
stop.

#### BACnetClient.discover_routers

```python
def discover_routers(
    self,
    network: Optional[int] = None,
    timeout_ms: int = 3000,
) -> Awaitable[list[DiscoveredRouter]]
```

Broadcast a Who-Is-Router-To-Network for `network` (or every
network) and collect the routers that answer for `timeout_ms`, one
per router MAC in the order first heard; for a `network`, only the
routers listing it. An empty list when none answer. `network` 0 or
65535 raises BacnetError (#1850). It holds the started client for
the whole wait, so `stop()` waits up to `timeout_ms` for it.

#### BACnetClient.network_reports

```python
def network_reports(self) -> Awaitable[NetworkReportIterator]
```

Follow the I-Am-Router-To-Network, Network-Number-Is and
Reject-Message-To-Network messages the client receives from now on
(#1850). RuntimeError before start and after stop.

#### BACnetClient.discovered_devices

```python
def discovered_devices(self) -> Awaitable[list[DiscoveredDevice]]
```

Return all devices discovered so far.

#### BACnetClient.get_device

```python
def get_device(self, instance: int) -> Awaitable[Optional[DiscoveredDevice]]
```

Look up a discovered device by instance number.

#### BACnetClient.clear_devices

```python
def clear_devices(self) -> Awaitable[None]
```

Clear the discovered device table.

#### BACnetClient.resolve_vmac

```python
def resolve_vmac(self, address: str) -> Awaitable[str]
```

Find the VMAC of the B/IPv6 node at `address`, given as
`"[ip]:port"`, and return it as `"bip6:0x0004D2"`, the form the
other methods take.

A B/IPv6 node is addressed by its VMAC, not its IPv6 address (Clause
U.1.3). This asks the node with a Virtual-Address-Resolution sent by
unicast, up to three times a second apart (Clause U.5), unless the
client already knows which VMAC is at that address. A link-local
address may carry its interface as a numeric zone
(`"[fe80::1%3]:47808"`).

Needs a started client with `transport="ipv6"`, else RuntimeError.
Any other address form raises ValueError. Raises BacnetTransportError
(errno ETIMEDOUT) when nothing answers within three seconds, or when
the node refuses.

#### BACnetClient.who_am_i

```python
def who_am_i(
    self,
    vendor_id: int,
    model_name: str,
    serial_number: str,
) -> Awaitable[None]
```

Broadcast a Who-Am-I request announcing this device's identity.

The three arguments should match the sender's Device object properties
(`vendor_id` is 0-65535). Raises `ValueError` if a string cannot be
encoded, or `OverflowError` for a `vendor_id` that doesn't fit.

#### BACnetClient.who_is_directed

```python
def who_is_directed(
    self,
    address: str,
    low_limit: Optional[int] = None,
    high_limit: Optional[int] = None,
) -> Awaitable[None]
```

Send a Who-Is to a specific device address (unicast).

Give `low_limit` and `high_limit` together or not at all, each from
0 to 4194303: one alone, `low_limit` above `high_limit`, or a limit
past 4194303 raises `ValueError` before anything is sent.

#### BACnetClient.time_synchronization

```python
def time_synchronization(
    self,
    address: str,
    date: tuple[int, int, int, int],
    time: tuple[int, int, int, int],
) -> Awaitable[None]
```

Send a TimeSynchronization request (unconfirmed).

`date` is `(year, month, day, day_of_week)`; `time` is
`(hour, minute, second, hundredths)`. The request sets a clock, so
both must be specific: a real day with the full year (1900..2154),
month 1..12, day 1..31 and `day_of_week` that day's own weekday
(1 = Monday), and every time field in range. A field that is
`UNSPECIFIED` (255) or a pattern value (an odd month, the last
day) raises ValueError before anything is sent.

#### BACnetClient.utc_time_synchronization

```python
def utc_time_synchronization(
    self,
    address: str,
    date: tuple[int, int, int, int],
    time: tuple[int, int, int, int],
) -> Awaitable[None]
```

Send a UTCTimeSynchronization request (unconfirmed); arguments as
for `time_synchronization`.

#### BACnetClient.read_property_from_device

```python
def read_property_from_device(
    self,
    device_instance: int,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    array_index: Optional[int] = None,
) -> Awaitable[PropertyValue]
```

Read a property from a device by instance number (auto-routing).

#### BACnetClient.read_property_multiple_from_device

```python
def read_property_multiple_from_device(
    self,
    device_instance: int,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]],
) -> Awaitable[Any]
```

Read multiple properties from a device by instance number (auto-routing).

#### BACnetClient.write_property_to_device

```python
def write_property_to_device(
    self,
    device_instance: int,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    value: PropertyValue,
    priority: Optional[int] = None,
    array_index: Optional[int] = None,
) -> Awaitable[None]
```

Write a property on a device by instance number (auto-routing).

Priority is None or 1..16. Invalid u8 priorities raise ValueError
synchronously before lookup; integers outside u8 raise OverflowError.

#### BACnetClient.write_property_multiple_to_device

```python
def write_property_multiple_to_device(
    self,
    device_instance: int,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]]]],
) -> Awaitable[None]
```

Write multiple properties to a device by instance number (auto-routing).

Same whole-request validation as write\_property\_multiple, synchronously
before discovery or dispatch: nonempty request/property lists, no special
selectors, priority None or 1..16. Invalid u8 priorities/structure raise
ValueError; priorities outside u8 raise OverflowError.

#### BACnetClient.add_device

```python
def add_device(self, device_instance: int, address: str) -> Awaitable[None]
```

Add a device to the discovery table manually (useful when address is known without WhoIs).

#### BACnetClient.subscribe_cov

```python
def subscribe_cov(
    self,
    address: str,
    subscriber_process_identifier: int,
    monitored_object_identifier: ObjectIdentifier,
    confirmed: bool,
    lifetime: Optional[int] = None,
) -> Awaitable[None]
```

Subscribe to Change-of-Value notifications for an object.

lifetime=None or 0 is indefinite; positive values are seconds.
Use unsubscribe\_cov for cancellation. confirmed always supplies the mode.

#### BACnetClient.unsubscribe_cov

```python
def unsubscribe_cov(
    self,
    address: str,
    subscriber_process_identifier: int,
    monitored_object_identifier: ObjectIdentifier,
) -> Awaitable[None]
```

Cancel a COV subscription.

#### BACnetClient.subscribe_cov_property_multiple

```python
def subscribe_cov_property_multiple(
    self,
    address: str,
    subscriber_process_identifier: int,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int], Optional[float], bool]]]],
    issue_confirmed_notifications: bool,
    max_notification_delay: Optional[int] = None,
    lifetime: Optional[int] = None,
) -> Awaitable[None]
```

Subscribe to COV notifications for multiple properties on multiple objects.

`specs` is `[(object_id, [(property_id, array_index, cov_increment, timestamped), ...]), ...]`.
`issue_confirmed_notifications` is required, including for cancellation requests.
For subscriptions and re-subscriptions, `lifetime` and `max_notification_delay` are both required.
For whole-context cancellations, pass an empty `specs` list and omit both fields.
Complete timing and nested-reference validation raises `ValueError` synchronously,
before an awaitable, address parsing, client-state access or I/O. Each supplied
object needs references; ALL/REQUIRED/OPTIONAL selectors are prohibited and the
cumulative reference limit is 10,000. Empty outer lists remain encodable with
valid finite timing, without claiming bundled-server empty-context support.
A device error about one COV reference raises BacnetProtocolError with
first\_failed\_subscription set; a general failure leaves it None.

#### BACnetClient.cov_notifications

```python
def cov_notifications(self) -> Awaitable[CovNotificationIterator]
```

Get an async iterator for incoming COV notifications.

#### BACnetClient.delete_object

```python
def delete_object(self, address: str, object_id: ObjectIdentifier) -> Awaitable[None]
```

Delete an object on a remote device (DeleteObject service).

#### BACnetClient.create_object

```python
def create_object(
    self,
    address: str,
    object_specifier: Union[ObjectType, ObjectIdentifier],
    initial_values: Optional[list[tuple[PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]]] = None,
) -> Awaitable[ObjectIdentifier]
```

Create an object on a remote device (CreateObject service).

`object_specifier` is an `ObjectType` (server assigns instance) or
`ObjectIdentifier` (specific instance). `initial_values` is
`[(property_id, value, priority, array_index), ...]`.
Returns the created object's identifier from the CreateObject-ACK; an
acknowledgment that doesn't decode raises BacnetError (the device may
still have created the object). A device error raises
BacnetProtocolError with first\_failed\_element\_number set when the
device answers with a CreateObject-Error.

#### BACnetClient.device_communication_control

```python
def device_communication_control(
    self,
    address: str,
    enable_disable: EnableDisable,
    time_duration: Optional[int] = None,
    password: Optional[str] = None,
) -> Awaitable[None]
```

Send DeviceCommunicationControl to a remote device.

#### BACnetClient.reinitialize_device

```python
def reinitialize_device(
    self,
    address: str,
    reinitialized_state: ReinitializedState,
    password: Optional[str] = None,
) -> Awaitable[None]
```

Send ReinitializeDevice to a remote device.

#### BACnetClient.acknowledge_alarm_request

```python
def acknowledge_alarm_request(
    self,
    address: str,
    acknowledging_process_identifier: int,
    event_object_identifier: ObjectIdentifier,
    event_state_acknowledged: EventState,
    timestamp: BACnetTimeStamp,
    acknowledgment_source: str,
    time_of_acknowledgment: BACnetTimeStamp,
) -> Awaitable[None]
```

Acknowledge an alarm with exact caller-supplied BACnetTimeStamp values.

`timestamp` must echo the original event notification timestamp;
`time_of_acknowledgment` is selected by the caller.

#### BACnetClient.acknowledge_alarm

```python
def acknowledge_alarm(
    self,
    address: str,
    acknowledging_process_identifier: int,
    event_object_identifier: ObjectIdentifier,
    event_state_acknowledged: EventState,
    acknowledgment_source: str,
) -> Awaitable[None]
```

Deprecated compatibility method.

This method fabricates sequence-number zero for both timestamps. Use
`acknowledge_alarm_request` for a lossless request.

#### BACnetClient.get_event_information

```python
def get_event_information(
    self,
    address: str,
    last_received_object_identifier: Optional[ObjectIdentifier] = None,
) -> Awaitable[bytes]
```

Get event information from a remote device. Returns raw response bytes.

#### BACnetClient.get_alarm_summary

```python
def get_alarm_summary(self, address: str) -> Awaitable[list[dict[str, Any]]]
```

Get alarm summary from a remote device.

Returns `[{"object_id": ObjectIdentifier, "alarm_state": EventState, "acknowledged_transitions": BitString}, ...]`, the bit string naming
`TO_OFFNORMAL`, `TO_FAULT` and `TO_NORMAL`.

#### BACnetClient.get_enrollment_summary

```python
def get_enrollment_summary(
    self,
    address: str,
    acknowledgment_filter: AcknowledgmentFilter = ...,
    event_state_filter: Optional[EnrollmentSummaryEventStateFilter] = None,
    event_type_filter: Optional[EventType] = None,
    min_priority: Optional[int] = None,
    max_priority: Optional[int] = None,
    notification_class_filter: Optional[int] = None,
) -> Awaitable[list[dict[str, Any]]]
```

Get enrollment summary from a remote device.

Returns `[{"object_id": ObjectIdentifier, "event_type": EventType, "event_state": EventState, "priority": int, "notification_class": Optional[int]}, ...]`.

#### BACnetClient.read_range

```python
def read_range(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    array_index: Optional[int] = None,
    range_type: Optional[Literal["position", "sequence", "time"]] = None,
    reference_index: Optional[int] = None,
    reference_seq: Optional[int] = None,
    count: Optional[int] = None,
    *,
    reference_time: ReferenceTime | None = None,
    validation: Literal["strict", "lenient"] = "strict",
) -> Awaitable[ReadRangeResult]
```

Read a range of items from a list or log object.

`range_type` is `"position"`, `"sequence"`, `"time"` or
`None` (all-items). `"time"` reads the `count` items newer than
`reference_time` (older, for a negative count): a naive
`datetime.datetime` in the device's local time, or a
`((year, month, day, day_of_week), (hour, minute, second, hundredths))` pair; an aware datetime raises ValueError, since the
device's zone isn't known here (#1533). Invalid selectors, array index
zero, missing or zero counts and a time that isn't specific raise
ValueError before I/O; a count outside INTEGER16 (-32768..=32767)
raises OverflowError before address parsing or I/O (#1360).
Position/sequence reference zero is valid; omitted references default
to zero.

`validation="strict"` refuses an answer that breaks a ReadRange rule
with BacnetReadRangeViolationError, whose `rule` names it;
`"lenient"` keeps it and lists the rules in `violations` (#1531).
Decode a log's records with `decode_log_records`.

#### BACnetClient.read_log_page

```python
def read_log_page(
    self,
    address: str,
    object_id: ObjectIdentifier,
    cursor: LogCursor = None,
    page_size: int = 100,
) -> Awaitable[LogPage]
```

Read one page of a log's Log\_Buffer from `cursor` (#1530).

`cursor` is `None` or `"oldest"` (the oldest record, found from
Record\_Count and Total\_Record\_Count), `("sequence", n)`,
`("position", n)` or `("time", reference_time)`; lists work
too, so a checkpoint survives JSON. Loop on `page["next"]` until
`page["done"]`, then keep `next` as the checkpoint. One request
is outstanding at a time. A first record past the one asked for sets
`gap`; when the log drops the record asked for, the read starts
over from the oldest. A page answered from before the one asked for,
or none where the counts say records are, raises
BacnetLogNotAdvancingError: such a device's sequence numbers are
inconsistent, so read it from `("position", 1)`. `wrapped` marks
a page that reached the top of the sequence range. A page whose
records don't decode raises BacnetError, dropping the records before
the failing one; an answer that breaks a rule the reader can't
tolerate, such as an echo that doesn't match, raises
BacnetReadRangeViolationError. A non-log object or a `page_size`
outside 1..=32767 raises ValueError before I/O.

#### BACnetClient.atomic_read_file

```python
def atomic_read_file(
    self,
    address: str,
    file_identifier: ObjectIdentifier,
    access_method: str,
    start_position: int = 0,
    requested_octet_count: int = 0,
    start_record: int = 0,
    requested_record_count: int = 0,
) -> Awaitable[AtomicReadFileResult]
```

Read from a file object. `access_method` is `"stream"` or `"record"`.
Returns the decoded acknowledgment: `access_method`, `end_of_file`,
then `start_position` and `data` for stream access, or
`start_record`, `record_count` and `records` for record access.
An acknowledgment in the other access method, or with more octets or
records than requested, raises BacnetError.

#### BACnetClient.atomic_write_file

```python
def atomic_write_file(
    self,
    address: str,
    file_identifier: ObjectIdentifier,
    access_method: str,
    start_position: int = 0,
    file_data: bytes = b"",
    start_record: int = 0,
    record_count: int = 0,
    file_record_data: Optional[list[bytes]] = None,
) -> Awaitable[AtomicWriteFileResult]
```

Write to a file object. `access_method` is `"stream"` or `"record"`.
Returns the decoded acknowledgment: `access_method`, then
`start_position` for stream access or `start_record` for record
access, where the device says the write took place. An
acknowledgment in the other access method raises BacnetError.

#### BACnetClient.add_list_element

```python
def add_list_element(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    list_of_elements: bytes,
    array_index: Optional[int] = None,
) -> Awaitable[None]
```

Add elements to a list property (AddListElement service).
Requires nonempty, completely framed element bytes and a nonzero optional
index. Invalid framing/index zero raises ValueError synchronously before
client access; indexes outside u32 raise OverflowError. Empty-valued,
constructed/context/vendor elements are retained without remote datatype
validation. Shared limits: 1 MiB per tag, 32 context levels including the
service wrapper; application Boolean has no payload bytes. A device
error raises BacnetProtocolError with first\_failed\_element\_number set
when the device answers with a ChangeList-Error.

#### BACnetClient.remove_list_element

```python
def remove_list_element(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    list_of_elements: bytes,
    array_index: Optional[int] = None,
) -> Awaitable[None]
```

Remove elements from a list property (RemoveListElement service).
Requires nonempty, completely framed element bytes and a nonzero optional
index. Invalid framing/index zero raises ValueError synchronously before
client access; indexes outside u32 raise OverflowError. Empty-valued,
constructed/context/vendor elements are retained without remote datatype
validation. Shared limits: 1 MiB per tag, 32 context levels including the
service wrapper; application Boolean has no payload bytes. A device
error raises BacnetProtocolError with first\_failed\_element\_number set
when the device answers with a ChangeList-Error.

#### BACnetClient.confirmed_private_transfer

```python
def confirmed_private_transfer(
    self,
    address: str,
    vendor_id: int,
    service_number: int,
    service_parameters: Optional[bytes] = None,
) -> Awaitable[dict[str, Any]]
```

Send a ConfirmedPrivateTransfer request.

Returns `{"vendor_id": int, "service_number": int, "result_block": bytes | None}`.
A device error raises BacnetProtocolError with vendor\_id,
service\_number and error\_parameters set from its
ConfirmedPrivateTransfer-Error. An ACK that is malformed, cut short
or has octets after its last member raises BacnetError.

#### BACnetClient.unconfirmed_private_transfer

```python
def unconfirmed_private_transfer(
    self,
    address: str,
    vendor_id: int,
    service_number: int,
    service_parameters: Optional[bytes] = None,
) -> Awaitable[None]
```

Send an UnconfirmedPrivateTransfer request.

#### BACnetClient.confirmed_text_message

```python
def confirmed_text_message(
    self,
    address: str,
    source_device: ObjectIdentifier,
    message_priority: MessagePriority,
    message: str,
    message_class_type: Optional[str] = None,
    message_class_value: Optional[Any] = None,
) -> Awaitable[None]
```

Send a ConfirmedTextMessage.

`message_class_type` is `"numeric"` or `"text"` (or `None` for no class).

#### BACnetClient.unconfirmed_text_message

```python
def unconfirmed_text_message(
    self,
    address: str,
    source_device: ObjectIdentifier,
    message_priority: MessagePriority,
    message: str,
    message_class_type: Optional[str] = None,
    message_class_value: Optional[Any] = None,
) -> Awaitable[None]
```

Send an UnconfirmedTextMessage.

#### BACnetClient.life_safety_operation

```python
def life_safety_operation(
    self,
    address: str,
    requesting_process_identifier: int,
    requesting_source: str,
    operation: LifeSafetyOperation,
    object_identifier: Optional[ObjectIdentifier] = None,
) -> Awaitable[None]
```

Send a LifeSafetyOperation request.

#### BACnetClient.write_group

```python
def write_group(
    self,
    address: Optional[str],
    group_number: int,
    write_priority: int,
    change_list: list[tuple[int, Optional[int], Union[PropertyValue, bytes, bytearray]]],
    inhibit_delay: Optional[bool] = None,
    *,
    network: Optional[int] = None,
) -> Awaitable[None]
```

Send a WriteGroup request (unconfirmed).

With an `address` the request goes to that one device. With
`address=None` it is broadcast: on the local network, or with
`network` on that remote network (1-65534), or on every network when
`network` is 65535. Give `network` only with `address=None`.

`group_number` is 1-4294967295 (group 0 is reserved) and
`write_priority` is 1-16. `change_list` is a non-empty list of
`(channel, override_priority_or_none, value)` tuples, where
`channel` is a channel number 0-65535, the override priority is 1-16 or
`None`, and `value` is a `PropertyValue`, which is encoded for you,
or a `bytes` or `bytearray` holding one encoded BACnetChannelValue.
Either way the value must be one BACnetChannelValue: a single
application-tagged primitive (`PropertyValue.real(72.0)`), or a
context-0 lighting command, context-1 xy colour or context-2 colour
command (as `bytes`, or the same octets in
`PropertyValue.application_data`), with no wrapper tag. Raises
`ValueError` for anything else, a list for one, `TypeError` for a
value of another type (a list of ints included), and `OverflowError`
for integers that don't fit.

#### BACnetClient.vt_open

```python
def vt_open(
    self,
    address: str,
    vt_class: VTClass,
    local_vt_session_identifier: int,
) -> Awaitable[int]
```

Open a virtual terminal session. Returns the remote session identifier.

`local_vt_session_identifier` (0-255) is the caller's own number for
the session, which the peer quotes when it sends data back.

#### BACnetClient.vt_close

```python
def vt_close(self, address: str, session_ids: list[int]) -> Awaitable[None]
```

Close one or more virtual terminal sessions.

`session_ids` must not be empty and each identifier is 0-255. Raises
`ValueError` for an empty list, or `OverflowError` for an integer that
doesn't fit, before anything is sent. A device error that lists the
sessions it could not close raises BacnetProtocolError with
vt\_session\_identifiers set.

#### BACnetClient.vt_data

```python
def vt_data(
    self,
    address: str,
    session_id: int,
    data: bytes,
    data_flag: bool,
) -> Awaitable[dict[str, Any]]
```

Send data over a virtual terminal session.

`data_flag` is the sequence flag that alternates on each new request
for a session; it is sent as an Unsigned 0 or 1.

Returns `{"all_new_data_accepted": bool, "accepted_octet_count": int | None}`;
the count is an `int` only when `all_new_data_accepted` is `False`.

#### BACnetClient.confirmed_audit_notification_typed

```python
def confirmed_audit_notification_typed(
    self,
    address: str,
    request: AuditNotificationRequestInput,
) -> Awaitable[None]
```

Send a validated mapping through the native confirmed Audit helper.

#### BACnetClient.unconfirmed_audit_notification_typed

```python
def unconfirmed_audit_notification_typed(
    self,
    address: str,
    request: AuditNotificationRequestInput,
) -> Awaitable[None]
```

Send a validated mapping through the native unconfirmed Audit helper.

#### BACnetClient.audit_log_query_typed

```python
def audit_log_query_typed(
    self,
    address: str,
    request: AuditLogQueryRequestInput,
) -> Awaitable[AuditLogQueryAck]
```

Send a typed Audit Log query and return a canonical decoded mapping.

#### BACnetClient.confirmed_audit_notification

```python
def confirmed_audit_notification(
    self,
    address: str,
    service_data: bytes,
) -> Awaitable[None]
```

Send a pre-encoded Clause 21 ConfirmedAuditNotification payload.

This is a raw escape hatch; the bundled server does not execute the
service.

#### BACnetClient.unconfirmed_audit_notification

```python
def unconfirmed_audit_notification(
    self,
    address: str,
    service_data: bytes,
) -> Awaitable[None]
```

Send a pre-encoded Clause 21 UnconfirmedAuditNotification payload.

This is a raw escape hatch; the bundled server does not execute the
service.

#### BACnetClient.audit_log_query

```python
def audit_log_query(self, address: str, service_data: bytes) -> Awaitable[bytes]
```

Send a pre-encoded Clause 21 AuditLogQuery payload.

This raw escape hatch returns the peer's response payload. The bundled
server does not execute the service.

#### BACnetClient.sc_npdu_drop_counts

```python
def sc_npdu_drop_counts(self) -> Awaitable[ScNpduDropCounts]
```

Copy SC ingress counters; requires a started SC owner.

Available while disconnected. Each call returns an independent dictionary;
a new transport starts fresh counters. No network I/O is performed.

#### BACnetClient.sc_peer_info

```python
def sc_peer_info(self) -> Awaitable[ScPeerInfo | None]
```

Observe the accepted active SC Hub without I/O; None while disconnected.
RuntimeError for non-SC, stopped or closing clients. Historical results
own data only. Observation time is not certificate validation time.

#### BACnetClient.read_bdt

```python
def read_bdt(self, address: str) -> Awaitable[list[tuple[str, str]]]
```

Read a BBMD's Broadcast Distribution Table as `("ip:port", "mask")` rows.

#### BACnetClient.write_bdt

```python
def write_bdt(
    self,
    address: str,
    entries: Sequence[str | tuple[str, str]],
) -> Awaitable[None]
```

Ask a BBMD to take `entries` (rows as `Bbmd(bdt=)` takes them)
as its BDT. A BBMD that follows the 2020 standard refuses with 0x0010.

#### BACnetClient.read_fdt

```python
def read_fdt(self, address: str) -> Awaitable[list[FdtEntry]]
```

Read a BBMD's Foreign Device Table.

#### BACnetClient.delete_fdt_entry

```python
def delete_fdt_entry(self, address: str, entry: str) -> Awaitable[None]
```

Ask a BBMD to delete the foreign device at `entry`
(`"ip:port"`) from its FDT; a refusal is 0x0050.

#### BACnetClient.register_foreign_device_bvlc

```python
def register_foreign_device_bvlc(self, address: str, ttl: int) -> Awaitable[None]
```

Register once as a foreign device for `ttl` seconds (0 to
65535); a refusal is 0x0030. Nothing is renewed and broadcasts don't
go through the BBMD: `foreign_device=` does that.

#### BACnetClient.connection_events

```python
def connection_events(self) -> Awaitable[ScEventIterator]
```

Subscribe to the SC hub-connector events (#1829). The first call
after a start gets the events since the client was built, starting
with the first `connected` (#1908); if that first call comes long
after the start, its first events may be old ones from the build. A
later call is a late subscriber and gets the events from that call
on. A slow reader never holds up the connector: see
`ScEventIterator.missed`.
RuntimeError for a stopped or non-SC client.

#### BACnetClient.connection_status

```python
def connection_status(self) -> Awaitable[ScLinkStatus]
```

Snapshot the SC hub link without I/O (#1829). RuntimeError for a
stopped or non-SC client.

#### BACnetClient.foreign_registration

```python
def foreign_registration(self) -> Awaitable[ForeignRegistrationStatus]
```

How the foreign-device registration is going (#938), a refusal
included. Sends nothing. Outside `foreign_device=` only manual
registrations show. RuntimeError before start and after stop; a
non-B/IP client raises BacnetError.

#### BACnetClient.stop

```python
def stop(self) -> Awaitable[None]
```

Refuse new work, drain operations, and join native teardown.

Cancelling this Future leaves shutdown running; call stop() again to
join it. Drain duration depends on existing requests. Errors retain
closing state for another stop attempt.

### ScReconnect

Frozen SC client retry settings, copied when passed to BACnetClient.

Delays must be nonzero, initial \<= maximum, and maximum \<= 86,400,000 ms.
Invalid settings raise ValueError; unsigned integer overflow raises
OverflowError. Zero retries allows the initial connection and disables
active-hub retries after loss. Validation is not deployment qualification.

Properties:

- `initial_delay_ms: int`
- `max_delay_ms: int`
- `max_retries: int`

#### ScReconnect.__init__

```python
def __init__(
    self,
    *,
    initial_delay_ms: int = 10000,
    max_delay_ms: int = 600000,
    max_retries: int = 10,
) -> None
```

### ScCertificateMetadata

Frozen informational metadata; ASCII-escaped fields are at most 1024 bytes.

Properties:

- `subject: str`
- `issuer: str`
- `not_before_unix: int`
- `not_after_unix: int`
- `public_key_algorithm_oid: str`
- `signature_algorithm_oid: str`
- `text_truncated: bool`
- `subject_alt_names: tuple[ScSubjectAltName, ...]`: The DNS, URI, email and IP Subject Alternative Names, in certificate order (#2022); other name forms are left out.

### ScSubjectAltName

One Subject Alternative Name exactly as the certificate wrote it:
no case folding, wildcards such as `*.example.com` as written. Compare
`value` for equality, never by suffix. Frozen; its repr shows the
kind only.

Properties:

- `kind: Literal["dns", "uri", "email", "ip"]`
- `value: str | ipaddress.IPv4Address | ipaddress.IPv6Address`: A `str`, or an `ipaddress` address for an `ip` name.

### ScInspectionFinding

Frozen local interpretation finding; never a TLS acceptance decision.

Properties:

- `area: Literal["certificate", "san", "key_usage", "extended_key_usage"]`
- `kind: Literal["malformed", "duplicate", "unsupported", "truncated"]`

### ScExtensionInspection

Frozen bounded display. Read certificate findings alongside these values.

Properties:

- `assessed: bool`
- `present: bool`
- `values: tuple[str, ...]`

### ScInspectedCertificate

Frozen exact DER fingerprint and optional interpretation, without retained DER.

Properties:

- `role: Literal["identity", "peer_trust"]`
- `index: int`
- `sha256: bytes`
- `metadata: ScCertificateMetadata | None`
- `validity: Literal["unavailable", "invalid_interval", "not_yet_valid", "expired", "within_interval"]`
- `expiry: Literal["not_assessed", "none", "warning", "alarm"]`
- `san: ScExtensionInspection`
- `key_usage: ScExtensionInspection`
- `extended_key_usage: ScExtensionInspection`
- `findings: tuple[ScInspectionFinding, ...]`

### ScKeyFilePermissions

Frozen Unix read-mode observations from the opened key file; no ACL audit.

Properties:

- `group_readable: bool`
- `world_readable: bool`

### ScPemInspection

Frozen owned offline data. No credential bytes, client or TLS policy is retained.

Properties:

- `inspected_at_unix: int`
- `peer_trust_supplied: bool`
- `certificates: tuple[ScInspectedCertificate, ...]`
- `key_consistency: Literal["not_assessed", "match", "mismatch", "unusable", "unknown"]`
- `key_file_permissions: ScKeyFilePermissions | None`
- `direct_issuer: Literal["not_assessed", "direct", "not_direct", "unknown"]`: Whether a `peer_trust` certificate signed the identity leaf directly (its subject is the leaf's issuer and the leaf's signature verifies with its key). `not_assessed` without peer trust; `unknown` when the leaf can't be parsed or a candidate's signature algorithm is unsupported. Informational: it changes no acceptance.
- `direct_issuer_index: int | None`: Zero-based `peer_trust` index of the CA that signed the leaf when `direct_issuer` is `direct`; else None.

### ScPeerCertificate

Frozen original DER and optional display interpretation.

Properties:

- `der: bytes`
- `metadata: ScCertificateMetadata | None`
- `inspection_error: Literal["certificate_parse_failed"] | None`

### ScUntrustedPeerInfo

Frozen, owned untrusted certificates from one failed outbound TLS attempt.

No accepted connection identity or verification result is implied. Missing
evidence does not prove that the peer sent no certificates. Metadata is
informational; retaining this value owns no network resources.

Properties:

- `peer_address: str`
- `server_name: str`
- `certificates: tuple[ScPeerCertificate, ...]`

### ScPeerInfo

Frozen historical observation; does not retain a client or socket.

Properties:

- `connection_id: int`
- `observed_at_unix: float`
- `peer_address: str`
- `server_name: str`
- `handshake_kind: Literal["full", "resumed", "unknown"]`
- `protocol_version: str | None`
- `cipher_suite: str | None`
- `leaf_sha256: bytes`
- `certificates: tuple[ScPeerCertificate, ...]`

### ScEvent

One SC hub-connector event (#1829), frozen. Fields that don't apply
to the event's kind are None.

A failover reads `disconnected`, then `reconnect_attempt` and
`connect_failed` for each failed primary attempt, then
`failover_engaged` and `connected` on the failover hub. A primary
restore reads `primary_restored`, then `connected` on the primary;
each failed restore probe, at most one per restore interval, reads
`primary_restore_failed` (#1828).

Properties:

- `kind: Literal['connected', 'disconnected', 'connect_failed', 'reconnect_attempt', 'failover_engaged', 'primary_restored', 'primary_restore_failed', 'reconnect_gave_up', 'other']`
- `hub: Literal["primary", "failover"] | None`: The hub connected to, lost, failed or being dialled; `primary` for `primary_restore_failed`.
- `hub_vmac: bytes | None`: The hub's VMAC from its Connect-Accept, on `connected`.
- `cause: Literal['local_close', 'peer_close', 'heartbeat_timeout', 'websocket', 'tls', 'nak', 'dial_timeout', 'connect_accept_timeout', 'upgrade_refused', 'malformed_result', 'unexpected_connect_reply', 'rejection_expired', 'other'] | None`: Why a `disconnected` link ended, or a `connect_failed` attempt or `primary_restore_failed` probe failed. `dial_timeout`: the dial (host lookup, TCP, TLS, WebSocket upgrade) ran out of time; `connect_accept_timeout`: the hub never answered the Connect-Request; `upgrade_refused`: the server answered the upgrade with `http_status` (#1839).
- `dial_stage: Literal['host_lookup', 'tcp_connect', 'tls', 'websocket_upgrade', 'other'] | None`: Where a `dial_timeout` dial stalled; None when it did not say.
- `http_status: int | None`: The HTTP status of an `upgrade_refused`, such as a proxy's 502.
- `cause_detail: str | None`: Diagnostic text for a `websocket`, `tls` or `other` cause; not stable.
- `error_class: ErrorClass | None`: The NAK's error class, for a `nak` cause.
- `error_code: ErrorCode | None`: The NAK's error code, for a `nak` cause.
- `attempt: int | None`: A `reconnect_attempt`'s number, from 1; the attempts made, for `reconnect_gave_up`.
- `backoff: float | None`: Seconds a `reconnect_attempt` waited first.

### ScLinkStatus

A frozen snapshot of the SC hub link (#1829).

Properties:

- `state: Literal["disconnected", "connecting", "connected", "disconnecting"]`
- `active_hub: Literal["primary", "failover"] | None`: The hub in use or, while reconnecting, being dialled; None before the first attempt.
- `hub_vmac: bytes | None`: The connected hub's VMAC; None while not connected.
- `heartbeat_age: float | None`: Seconds since the last message from the hub; None while not connected.
- `heartbeat_rtt: float | None`: Round-trip seconds of the latest answered Heartbeat-Request on this connection; None until one is answered.

### ScEventIterator

Async iterator over `ScEvent` from one `connection_events()` call.

The connector never waits for it: an iterator more than 64 events behind
loses the oldest and adds them to `missed`. A cancel that lands while an
event is being delivered, as a timed-out `asyncio.wait_for` does, puts
the event back: the next `__anext__` returns it first. Concurrent
`__anext__` calls complete one at a time. Read it from one running event
loop (`asyncio.run` is fine): an item on its way to a read whose loop
stops for good holds up later reads, which raise one `RuntimeWarning`
after 5 s and keep waiting. Iteration ends when the client stops.

Properties:

- `missed: int`: Events lost so far: because this iterator fell behind the connector, or dropped on the way to a read, as when its event loop closed first.

#### ScEventIterator.__aiter__

```python
def __aiter__(self) -> ScEventIterator
```

#### ScEventIterator.__anext__

```python
def __anext__(self) -> Awaitable[ScEvent]
```

## Server

The server, its policies and counters, and the BACnet/SC hub.

### BACnetServer

BACnet server that hosts objects and responds to client requests.

For SC, sc\_ca\_cert, sc\_client\_cert, and sc\_client\_key must be nonempty
paths; omission, None, or empty strings raise ValueError at construction.
Their None defaults preserve non-SC use and positional layout only.
start() loads the explicit site CA and operational cert/key before dialing
or draining registrations. Local TLS configuration errors raise RuntimeError;
repair the files and retry on the same server. Later startup failures do not
have a general registration rollback guarantee.

SC also requires keyword-only sc\_device\_uuid: exactly 16 bytes, not all zero,
copied at construction. Missing, None, wrong length, or all-zero values
raise ValueError after credential-presence validation, before file/network I/O.
Provision before deployment and durably reuse the same UUID for the device's
lifetime. No generation, persistence, change detection, or version/variant
enforcement is provided. Non-SC transports ignore this option.

Segmentation defaults to NONE. When enabled, apdu\_segment\_timeout\_ms must
be positive; it supplies Device APDU\_SEGMENT\_TIMEOUT, response SegmentACK
waits, and the 4\*Tseg receive inactivity timer. A separate 16-second
no-progress resource limit aborts abandoned request transfers with OTHER.

Usage:

```python
server = BACnetServer(device_instance=1234, device_name="My Device")
server.add_analog_input(1, "Temperature", units=62)
await server.start()
```

#### BACnetServer.__init__

```python
def __init__(
    self,
    device_instance: int,
    device_name: str = "BACnet Device",
    interface: str = "0.0.0.0",
    port: int = 0xBAC0,
    broadcast_address: str = "255.255.255.255",
    transport: str = "bip",
    sc_hub: Optional[str] = None,
    sc_vmac: Optional[bytes] = None,
    sc_ca_cert: Optional[str] = None,
    sc_client_cert: Optional[str] = None,
    sc_client_key: Optional[str] = None,
    sc_heartbeat_interval_ms: Optional[int] = None,
    sc_heartbeat_timeout_ms: Optional[int] = None,
    ipv6_interface: Optional[str] = None,
    dcc_password: Optional[str] = None,
    reinit_password: Optional[str] = None,
    *,
    mutation_policy: Literal["permissive", "deny_all"] | MutationHandler = "permissive",
    dcc_policy: str = "deny_all",
    dcc_source_restriction: list[tuple[int | None, bytes]] | None = None,
    dcc_disable_rate_limit: tuple[int, int] | None = None,
    serial_port: Optional[str] = None,
    mstp_baud: int = 38400,
    mstp_mac: int = 1,
    mstp_max_master: int = 127,
    mstp_max_info_frames: int = 1,
    mstp_extended_frames: bool = False,
    mstp_echo: bool = False,
    mstp_echo_grace_ms: Optional[int] = None,
    mstp_transmit_latency_pad_ms: Optional[int] = None,
    mstp_usage_timeout_ms: Optional[int] = None,
    mstp_reply_timeout_ms: Optional[int] = None,
    max_confirmed_in_flight: int = 64,
    max_unconfirmed_in_flight: int = 32,
    max_confirmed_in_flight_per_peer: int = 16,
    max_unconfirmed_in_flight_per_peer: int = 8,
    confirmed_recovery_reserve: int = 4,
    max_recovery_in_flight_per_peer: int = 1,
    rpm_max_result_elements: int = 256,
    rpm_max_service_ack_bytes: int = 16384,
    alarm_summary_max_objects: int = 4096,
    alarm_summary_max_service_ack_bytes: int = 16384,
    enrollment_summary_max_objects: int = 4096,
    enrollment_summary_max_service_ack_bytes: int = 16384,
    atomic_read_file_max_requested_stream_octets: int = 16384,
    atomic_read_file_max_requested_records: int = 256,
    atomic_read_file_max_service_ack_bytes: int = 16384,
    atomic_write_file_max_stream_payload_octets: int = 16384,
    atomic_write_file_max_records: int = 256,
    atomic_write_file_max_record_payload_bytes: int = 16384,
    read_range_max_returned_items: int = 256,
    read_range_max_service_ack_bytes: int = 16384,
    event_information_max_objects: int = 4096,
    event_information_max_returned_summaries: int = 256,
    event_information_max_service_ack_bytes: int = 16384,
    sc_device_uuid: Optional[bytes | bytearray] = None,
    registered_network_port: Optional[int] = None,
    cov_policy: CovPolicy | None = None,
    time_sync_policy: TimeSyncPolicy | None = None,
    share_port_by_address: bool = False,
    segmentation_supported: Segmentation = Segmentation.NONE,
    apdu_segment_timeout_ms: int = 5000,
    bbmd: Optional[Bbmd] = None,
    foreign_device: Optional[ForeignDevice] = None,
    multicast: Optional[Multicast] = None,
    sc_npdu_per_origin_limit: int | None = None,
    sc_control_rate_limit: tuple[int, int] | None = None,
    on_reinitialize: Optional[ReinitializeHandler] = None,
    after_reinitialize: Optional[AfterReinitialize] = None,
) -> None
```

`dcc_password` and `reinit_password` hold at most 20
characters; a longer one raises ValueError here (#2060).

`on_reinitialize` (default None): carry out ReinitializeDevice
(#1566). It is called with a `ReinitializeRequest` once a request
has passed its password and state checks, while the server holds its
object database, so it must return at once: no awaiting, no blocking,
and no other `rusty_bacnet` call (one would raise RuntimeError).
That check sees only its own thread: never wait on the event loop or
another thread from it (`run_coroutine_threadsafe(...).result()`,
`Thread.join`, any future's `.result()`), which deadlocks the
server. An `async def` handler is answered SERVICES / OTHER.
Return None or True to accept, False to refuse with SERVICES /
SERVICE\_REQUEST\_DENIED, or an `(ErrorClass, ErrorCode)` tuple to
refuse with those. An exception, or any other return value, answers
SERVICES / OTHER and goes to `sys.unraisablehook`. Without it every
request is refused. With it, a registered BBMD port's BDT and
foreign-device rows take writes, which ACTIVATE\_CHANGES or WARMSTART
puts into effect.

`after_reinitialize` (default None; needs `on_reinitialize`):
called with the `ReinitializeRequest` on the event loop that
started the server, once the accepted request's SimpleACK has been
handed to the transport. Restart, back up or restore here. It never
runs for a refused request, nor after a `stop()` that began before
the hand-off. A coroutine it returns runs as a task. On MS/TP, handed
off means queued: the frame still waits for the token, so stopping
at once can drop the reply; wait a short while first.

`share_port_by_address` (B/IP only, default False): bind the
interface address itself, so devices on other addresses of this host
can share the port. Needs an explicit interface and a nonzero port;
broadcasts and unicast are still handled in the order they arrived.

`bbmd` / `foreign_device` (at most one, else ValueError): run
the device as a BBMD (B/IP only), or register it with a BBMD on
another subnet as a foreign device (#938). `foreign_device` also
applies to `transport="ipv6"` with a B/IPv6 BBMD (#1941). See
`Bbmd` and `ForeignDevice`. A registered Network Port reports
the mode in BACnet\_IP\_Mode (normal 0, foreign 1, BBMD 2), or on
B/IPv6 in BACnet\_IPv6\_Mode (normal 0, foreign 1).

`registered_network_port` (B/IP or B/IPv6, 1 to 255): the Network
Port object, added with `add_bip_network_port` (`transport="bip"`)
or `add_bip6_network_port` (`transport="ipv6"`), that represents
the server's link. Start checks the object's address and UDP port
against the transport's, then publishes the bound port. A B/IPv6
port also publishes its VMAC and multicast group, and reports
BACnet\_IPv6\_Mode NORMAL (0), or FOREIGN (1) with `foreign_device=`
and its FD\_BBMD\_Address and FD\_Subscription\_Lifetime rows. With
`on_reinitialize`, a client may write those two rows; a
ReinitializeDevice ACTIVATE\_CHANGES then registers with the new
values at once.

`sc_npdu_per_origin_limit` (SC only, default None): maximum
queued, unconsumed NPDUs per ingress path and source. None keeps 32;
explicit values must be 1 through 256. Invalid values or a non-SC
transport raise ValueError before file or network I/O. Negative or
unrepresentably large values raise OverflowError. The aggregate
receive queue remains 256; see `sc_npdu_drop_counts`.

`sc_control_rate_limit` (SC only, default None): `(burst, per_second)`, the network-layer messages one source may send back to
back and then each second (#1969). None keeps 256 then 32 a second.
The burst must be 1 through 4096 and the rate at least 1; invalid
values or a non-SC transport raise ValueError before file or network
I/O, and negative values raise OverflowError. A message over its
source's budget is dropped and counted in `control_rate_drops`.

`mstp_extended_frames` (MS/TP only, default False): take and send
extended frames (Types 32 and 33), which carry NPDUs of up to 1497
octets, so the device accepts APDUs of up to 1476 instead of 480.

`mstp_echo` and `mstp_echo_grace_ms` (MS/TP only) declare an
adapter that returns every octet this station sends, as for
`BACnetClient`; a negative grace, or one above 4294967295, raises
OverflowError when the server is built.

`mstp_transmit_latency_pad_ms`, `mstp_usage_timeout_ms` and
`mstp_reply_timeout_ms` (MS/TP only) set the transmit-latency pad
and the two timeouts, in milliseconds, as for `BACnetClient`.

#### BACnetServer.add_analog_input

```python
def add_analog_input(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    present_value: float = 0.0,
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

`tags`, `profile_location` and `profile_name` provision the
optional Tags, Profile\_Location and Profile\_Name rows; each is served
only when given. Tags takes writes over the network, the other two
are read-only. A tag name with a semicolon, a tag value that isn't
primitive, a location whose URI scheme isn't http, https or bacnet,
or a profile name without its vendor-number prefix (`"555-room"`)
raises BacnetProtocolError, and wrong shapes TypeError or
ValueError.

#### BACnetServer.add_analog_output

```python
def add_analog_output(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Profile keywords as on `add_analog_input`.

#### BACnetServer.add_analog_value

```python
def add_analog_value(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Optional AV Audit rows: None is absent; priority 'inherit' is present NULL.

`present_value_access` is `"commandable"` (writes go through
Priority\_Array), `"writable"` (a write replaces Present\_Value) or
`"read_only"` (peers write it only while out of service).
`value_source_tracking` serves Value\_Source on a noncommandable
object: the writer of the last Present\_Value write, who alone may
then correct it. A commandable object tracks its sources through
Priority\_Array already, so asking it to track raises ValueError.
Profile keywords as on `add_analog_input`.

#### BACnetServer.add_binary_input

```python
def add_binary_input(
    self,
    instance: int,
    name: str,
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Profile keywords as on `add_analog_input`.

#### BACnetServer.add_binary_output

```python
def add_binary_output(
    self,
    instance: int,
    name: str,
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Profile keywords as on `add_analog_input`.

#### BACnetServer.add_binary_value

```python
def add_binary_value(
    self,
    instance: int,
    name: str,
    *,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Optional BV Audit rows: None is absent; priority 'inherit' is present NULL.
Access, Value\_Source and profile keywords as on `add_analog_value`.

#### BACnetServer.add_multistate_input

```python
def add_multistate_input(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Profile keywords as on `add_analog_input`.

#### BACnetServer.add_multistate_output

```python
def add_multistate_output(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Profile keywords as on `add_analog_input`.

#### BACnetServer.add_multistate_value

```python
def add_multistate_value(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Access, Value\_Source and profile keywords as on `add_analog_value`.

#### BACnetServer.add_calendar

```python
def add_calendar(
    self,
    instance: int,
    name: str,
    *,
    date_list: Optional[list[CalendarEntry]] = None,
) -> None
```

`date_list` seeds Date\_List with calendar entries, the form a typed
read of it gives. An entry the Calendar refuses, such as a month
outside 1 to 14 or 255, raises BacnetProtocolError (VALUE\_OUT\_OF\_RANGE)
and nothing is registered; wrong shapes raise TypeError or
ValueError.

#### BACnetServer.add_schedule

```python
def add_schedule(
    self,
    instance: int,
    name: str,
    *,
    weekly_schedule: Optional[list[list[TimeValue]]] = None,
    exception_schedule: Optional[list[SpecialEvent]] = None,
    effective_period: Optional[tuple[BACnetDate, BACnetDate]] = None,
) -> None
```

Add a Schedule object; Schedule\_Default is NULL.

The keywords take the forms typed reads of the properties give, so a
read's `.value` builds the same Schedule: `weekly_schedule` seven
days, Monday first, each a list of `(time, value)` pairs;
`exception_schedule` a list of `SpecialEvent` mappings;
`effective_period` a `(start_date, end_date)` pair, each date
specific or wholly unspecified (the default covers every date). What
the Schedule refuses, such as a time given twice in a day, a value
that isn't primitive or a priority outside 1 to 16, raises
BacnetProtocolError and nothing is registered.

#### BACnetServer.add_date_value

```python
def add_date_value(self, instance: int, name: str) -> None
```

#### BACnetServer.add_time_value

```python
def add_time_value(self, instance: int, name: str) -> None
```

#### BACnetServer.add_date_time_value

```python
def add_date_time_value(self, instance: int, name: str) -> None
```

#### BACnetServer.add_date_pattern_value

```python
def add_date_pattern_value(self, instance: int, name: str) -> None
```

#### BACnetServer.add_time_pattern_value

```python
def add_time_pattern_value(self, instance: int, name: str) -> None
```

#### BACnetServer.add_date_time_pattern_value

```python
def add_date_time_pattern_value(self, instance: int, name: str) -> None
```

#### BACnetServer.add_notification_class

```python
def add_notification_class(
    self,
    instance: int,
    name: str,
    notification_class: int = 0,
    storage_path: Optional[str] = None,
    *,
    recipients: Optional[list[Destination]] = None,
) -> None
```

Add a Notification Class (Clause 12.21). With `storage_path`, a
Recipient\_List a client writes is kept in that file across restarts; a
write whose list cannot be saved is refused with DEVICE /
OPERATIONAL\_PROBLEM and the old list stays. Without it the list lives in
memory only.

`storage_path` is a `str`; a `pathlib.Path` raises TypeError, as
for `add_notification_forwarder`. Give each class a file of its own:
the file names the class it belongs to, so one that holds another
object's list, or anything this backend did not write, makes this call
raise BacnetError (BacnetProtocolError for a saved list a client's write
would be refused).

`recipients` seeds Recipient\_List with `Destination` mappings, in
order, with the checks and errors `add_notification_forwarder`'s
`recipients` has: more than 32, or an address MAC past 18 octets,
raises BacnetProtocolError, and nothing is registered. With
`storage_path`, a Recipient\_List a client wrote, once saved, wins:
until a write sets the list, the seed applies at every start and is
not saved.

#### BACnetServer.add_notification_forwarder

```python
def add_notification_forwarder(
    self,
    instance: int,
    name: str,
    process_identifier_filter: Optional[int] = None,
    local_forwarding_only: bool = False,
    storage_path: Optional[str] = None,
    *,
    recipients: list[Destination] | None = None,
    port_filter: list[tuple[int, bool]] | None = None,
) -> None
```

Add a Notification Forwarder (Clause 12.51) that sends the event
notifications this server receives on to its Recipient\_List and
Subscribed\_Recipients. `process_identifier_filter=None` forwards every
process identifier. With `storage_path`, Recipient\_List and
Subscribed\_Recipients are kept in that file across restarts.

`recipients` seeds Recipient\_List in order; more than 32 destinations,
or an address MAC past 18 octets, raises BacnetProtocolError, as a
client's write would be refused. With `storage_path`, a
Recipient\_List a client wrote, once saved, wins over the seed. `port_filter` serves Port\_Filter as
`(port_id, enabled)` pairs; the server receives through Port\_ID 0.
Without it Port\_Filter is absent.

#### BACnetServer.add_trend_log

```python
def add_trend_log(
    self,
    instance: int,
    name: str,
    buffer_size: int = 100,
    *,
    reference: tuple[ObjectIdentifier, PropertyIdentifier] | tuple[ObjectIdentifier, PropertyIdentifier, Optional[int]] | DeviceObjectPropertyReference | None = None,
    log_interval: int | None = None,
    logging_type: Literal["polled", "triggered", "cov"] | None = None,
    client_cov_increment: float | None = None,
    cov_resubscription_interval: int | None = None,
    start_time: tuple[tuple[int, int, int, int], tuple[int, int, int, int]] | None = None,
    stop_time: tuple[tuple[int, int, int, int], tuple[int, int, int, int]] | None = None,
    align_intervals: bool | None = None,
    interval_offset: int | None = None,
    storage_path: Optional[str] = None,
    total_record_count: int = 0,
) -> None
```

Add a Trend Log (Clause 12.25) that the server polls, triggers or
logs by COV (before starting).

`reference` sets Log\_DeviceObjectProperty, one element in any form
`add_trend_log_multiple` takes for `members`. `log_interval`,
`start_time`, `stop_time`, `align_intervals` and
`interval_offset` are as there. `logging_type` `"polled"` with
no `log_interval` takes a one-minute interval; `"triggered"`
zeroes Log\_Interval and makes it read-only, so a `log_interval`
with it raises BacnetProtocolError (WRITE\_ACCESS\_DENIED); `"cov"`
zeroes it too, and a nonzero `log_interval` then turns the log back
to polled (Clause 12.25.9). Any other string raises ValueError, and
so does `"polled"` with `log_interval=0`, a pair that would make a
COV log; `log_interval=0` alone leaves the new log polled and idle.

While COV, the poller looks at the referenced property of this
device on each pass and logs it, with its object's Status\_Flags,
whenever it changes as a COV notification would report it. Enable
turned off and on again within one pass (about 100 ms) isn't seen.
`client_cov_increment` (a REAL) is the increment a numeric value is
held to; None, the default, is NULL: the object's own COV\_Increment
for a numeric Present\_Value, any change otherwise. A negative or
non-finite one raises BacnetProtocolError (VALUE\_OUT\_OF\_RANGE).
`cov_resubscription_interval` is in seconds, nonzero; it is served
but unused, since the server doesn't subscribe to other devices: a
reference to one logs a single OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED
failure.

Log\_DeviceObjectProperty reads as one
`device_object_property_reference` value whose mapping includes all
four keys; while unset its object is Analog Input
4194303's Present\_Value, and writing a reference to instance 4194303
unsets it. A null written to it succeeds and changes nothing.

`total_record_count` seeds Total\_Record\_Count, an Unsigned32, so
the log's first record is numbered one past it: seeded at
4294967294, records number 4294967295, then 1, 2 and on, which lets
a reader's handling of the wrap be tested. Out of range raises
OverflowError.

`storage_path` keeps the log in files, as on
`add_trend_log_multiple`. `client_cov_increment` and
`cov_resubscription_interval` aren't saved: they apply on every
start, reopened or not.

#### BACnetServer.add_trend_log_multiple

```python
def add_trend_log_multiple(
    self,
    instance: int,
    name: str,
    buffer_size: int = 100,
    *,
    members: Optional[list[tuple[ObjectIdentifier, PropertyIdentifier] | tuple[ObjectIdentifier, PropertyIdentifier, Optional[int]] | DeviceObjectPropertyReference]] = None,
    log_interval: int | None = None,
    logging_type: Literal["polled", "triggered"] | None = None,
    start_time: tuple[tuple[int, int, int, int], tuple[int, int, int, int]] | None = None,
    stop_time: tuple[tuple[int, int, int, int], tuple[int, int, int, int]] | None = None,
    align_intervals: bool | None = None,
    interval_offset: int | None = None,
    storage_path: Optional[str] = None,
    total_record_count: int = 0,
) -> None
```

Add a Trend Log Multiple (Clause 12.30) that the server polls or
triggers, logging one value per member in each record.

`members` fills Log\_DeviceObjectProperty in order, each an
`(object, property)` or `(object, property, array_index)` tuple
or a `DeviceObjectPropertyReference` mapping; one naming this
server's Device is kept in its local form. More than 64 raises
BacnetProtocolError (NO\_SPACE\_TO\_WRITE\_PROPERTY), and a
`device_identifier` that isn't a Device raises ValueError.
`log_interval` is in hundredths of a second. `logging_type`
`"polled"` with no `log_interval` takes a one-minute interval;
`"triggered"` zeroes Log\_Interval and makes it read-only, so a
`log_interval` with it raises BacnetProtocolError
(WRITE\_ACCESS\_DENIED). `"cov"` raises BacnetProtocolError
(VALUE\_OUT\_OF\_RANGE), as a client's write of COV does, since a Trend
Log Multiple never logs by COV; any other string raises ValueError. With neither, Log\_Interval stays 0 and
nothing is polled.

`start_time` and `stop_time` bound when records are kept, each a
`((full_year, month, day, day_of_week), (hour, minute, second, hundredths))` pair: every field 255 leaves that side open, 255
seconds or hundredths count as zero, and anything else that isn't an
actual date and time raises BacnetProtocolError (VALUE\_OUT\_OF\_RANGE). Records are kept from the
start up to, not including, the stop, and each opening and closing
is logged. `align_intervals` aligns a polled log's acquisitions to
the clock when Log\_Interval divides a day, shifted by
`interval_offset` hundredths (modulo Log\_Interval).

Peers can write each of these. To ask a triggered log for one
record, write Trigger TRUE, from a peer or with
`write_property_local`; it reads TRUE until the poller acquires
the record. Read the records with `read_range`.

`total_record_count` seeds Total\_Record\_Count as on
`add_trend_log`.

`storage_path` keeps the log in `<storage_path>.slot0` and
`.slot1`. It is a `str`; a `pathlib.Path` raises TypeError. The
log commits a snapshot every 100 records or, while it changes, once a
minute, and once more at `stop()`, so a power loss costs at most
about a minute of records; a failed commit is retried. A log reopened
from those files serves its saved records, Total\_Record\_Count,
BUFFER\_READY counts and Notification\_Threshold, Enable,
Stop\_When\_Full and every setting a peer can write, and appends one
LOG\_INTERRUPTED record to mark the restart (none when it reopens full
and stopped); the saved state wins over the keyword arguments.
`buffer_size` wins over the saved Buffer\_Size: a smaller one keeps
the newest records. A durable Trend Log Multiple holds at most 15849
records (178437 for an Event Log, 828302 for a Trend Log); a larger
`buffer_size`, or a file pair that can't be read or holds another
object's state, raises BacnetError. To tell whether a log will
reopen, check whether either slot file exists before the call. The
files outlive the log: one added again on them reopens.

#### BACnetServer.add_event_log

```python
def add_event_log(
    self,
    instance: int,
    name: str,
    buffer_size: int = 100,
    log_received_notifications: bool = False,
    *,
    storage_path: Optional[str] = None,
    total_record_count: int = 0,
) -> None
```

Register an Event Log of `buffer_size` records before start().

The log records each event notification the server builds. With
`log_received_notifications` it also records the Confirmed and
UnconfirmedEventNotifications the server receives, unicast or
broadcast, as they decoded; each source is held to a few records a
second, and the rest show in `event_notification_counters()` as
`received_not_logged`.

The log reports BUFFER\_READY to its Notification Class once its
Notification\_Threshold is set (zero, the default, reports nothing);
write it and Notification\_Class with `write_property_local` or
from a peer.

`total_record_count` seeds Total\_Record\_Count as on
`add_trend_log`; BUFFER\_READY counts from the seeded value.

`storage_path` keeps the log in files, as on
`add_trend_log_multiple`. `log_received_notifications` isn't
saved and applies on every start.

#### BACnetServer.add_audit_log

```python
def add_audit_log(
    self,
    instance: int,
    name: str,
    storage_path: str,
    buffer_size: int = 100,
) -> None
```

#### BACnetServer.add_device_binding

```python
def add_device_binding(self, device_instance: int, address: str) -> None
```

Register a direct B/IP (IPv4) Device binding on a transport="bip" server.

Accept IPv4:port or exactly six hex bytes (four IPv4 octets and two port octets).
Other transports or parsed address lengths raise ValueError without retention.
The shared parser's broader grammar remains available to other APIs, not bindings.
Invalid instance/address or duplicate Device raises ValueError. No overwrite,
routing or discovery. Syntax validation precedes the frozen-state check; after
start() consumes configuration, parseable calls raise RuntimeError before the
transport/shape checks, including after stop. start() refuses a binding at a
broadcast or other group address of the link (a multicast address,
255.255.255.255, or the broadcast IP at another port), raising BacnetError
naming the device and the address; bind each device at its unicast address.

#### BACnetServer.configure_audit_log_parent

```python
def configure_audit_log_parent(
    self,
    instance: int,
    *,
    parent_device_instance: int,
    parent_audit_log_instance: int,
) -> None
```

Set a registered Audit Log's remote parent; valid pre-start calls replace it.

Identifiers are integers in 0..=4194303, not bool. Invalid/missing/duplicate
local logs or a local parent Device raise ValueError without mutation.
Startup revalidates local identity; settings freeze at ownership transfer.
Reapply on a new server after reopen. Only the selected inbound sink forwards,
using a configured direct B/IP binding with the existing one-attempt confirmed
policy, never deletion or a durable queue. IPv6/SC/MS/TP forwarding is not exposed.

#### BACnetServer.configure_audit_notification_sink

```python
def configure_audit_notification_sink(
    self,
    instance: int,
    *,
    policy: Literal["deny_all", "allow_all"],
) -> None
```

Select one registered Audit Log before start(); omission denies receipt.

Invalid policy/instance, missing or duplicate log raises ValueError without
mutation. Startup revalidates before transport preparation/registration
transfer. A running server raises RuntimeError. Payload Source\_Device and
Target\_Device remain peer-reported, not verified origin. No Python callbacks.

#### BACnetServer.add_audit_reporter

```python
def add_audit_reporter(self, instance: int, name: str) -> None
```

#### BACnetServer.configure_audit_recipient

```python
def configure_audit_recipient(self, recipient: AuditRecipientInput) -> None
```

Provision the copied Device-owned recipient before startup.

Accepts a concrete remote Device (not instance 4194303), or a supported
direct unicast IPv4 BACnetAddress on B/IP. The selected target Reporter
requires provision even at Audit\_Level NONE. Add Device routes with
add\_device\_binding independently. Missing Device routes start with
CONFIGURATION\_ERROR; live changes require usable old and new routes.
This setter freezes at ownership transfer. Use Device property writes
after start for atomic old/new delivery. No NULL recipient sentinel.

#### BACnetServer.configure_audit_reporters

```python
def configure_audit_reporters(self, reporters: list[AuditReporterConfiguration]) -> None
```

Replace the complete target Reporter set before start (one through 64).

Each configuration identifies a registered concrete Reporter (0..4194302).
Invalid instance values (including bool and non-int), duplicate identities,
invalid settings or missing objects raise ValueError. Wrong container types
and wrong types for other setting fields raise TypeError. The entire input
is copied and validated before pending changes. Startup freezes this API.
Omitted optional fields reset to catch-all selectors, all priorities and
absent delay/control properties. maximum\_send\_delay accepts None or an
integer 0..3600: zero exposes immediate Maximum\_Send\_Delay/Send\_Now;
positive values enable bounded ordinary target batching. Bool/non-integer
delay raises TypeError; an integer outside unsigned32 raises
OverflowError and one past 3600 ValueError. An `instance`,
`auditable_operations` or `audit_priority_filter` outside its
integer type (unsigned32, unsigned64, unsigned16) raises OverflowError
too.
Enabled nominal overlaps expose CONFIGURATION\_ERROR on every affected
Reporter's RELIABILITY; the lowest instance emits, before operation filters.
Empty/all-None selectors select no nominal targets. Mandatory Reporter
writes and Device recipient changes have separate bounded capture rules.
All Reporters use the one provisioned Device recipient and share the same
64-operation admission budget. Source reporting remains exactly one.

#### BACnetServer.add_loop

```python
def add_loop(
    self,
    instance: int,
    name: str,
    output_units: EngineeringUnits | int = 62,
    *,
    controlled_variable_units: Optional[EngineeringUnits | int] = None,
    proportional_constant_units: Optional[EngineeringUnits | int] = None,
    integral_constant_units: Optional[EngineeringUnits | int] = None,
    derivative_constant_units: Optional[EngineeringUnits | int] = None,
    priority_for_writing: Optional[int] = None,
) -> None
```

Add a Loop (PID) object to the server (before starting).

The keyword arguments set rows that are read-only over the network:
Controlled\_Variable\_Units and the three gain units rows (NO\_UNITS when
omitted) and Priority\_For\_Writing (16 when omitted). Units above 65535
or a priority outside 1..=16 raise BacnetProtocolError with
VALUE\_OUT\_OF\_RANGE; a priority outside 0..=255 raises OverflowError.
Peers can write Action (DIRECT until written).

Controlled\_Variable\_Reference and Manipulated\_Variable\_Reference read
as `application_data` holding the context-tagged reference; while
unset they name instance 4194303 (an Analog Input and an Analog Output
respectively), and writing a reference to that instance unsets them.
A null written to any Loop reference succeeds and changes nothing.

#### BACnetServer.add_command

```python
def add_command(
    self,
    instance: int,
    name: str,
    *,
    action: Optional[list[list[ActionCommand]]] = None,
    action_text: Optional[list[str]] = None,
) -> None
```

Add a Command object; writing N to its Present\_Value runs action\[N-1\].

`action_text` serves Action\_Text and needs one text per list. A
command whose `device_identifier` isn't a Device raises ValueError.
A priority outside 1..=16, a value with no encoding, or a text count
that differs from the list count raises BacnetProtocolError with
VALUE\_OUT\_OF\_RANGE.

#### BACnetServer.add_timer

```python
def add_timer(self, instance: int, name: str) -> None
```

#### BACnetServer.add_load_control

```python
def add_load_control(self, instance: int, name: str) -> None
```

#### BACnetServer.add_program

```python
def add_program(self, instance: int, name: str) -> None
```

#### BACnetServer.add_lighting_output

```python
def add_lighting_output(
    self,
    instance: int,
    name: str,
    *,
    high_end_trim: Optional[float] = None,
    low_end_trim: Optional[float] = None,
    trim_fade_time: Optional[int] = None,
    color_reference: Optional[ObjectIdentifier] = None,
    color_override: Optional[bool] = None,
    override_color_reference: Optional[ObjectIdentifier] = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Add a Lighting Output object.

Its Lighting\_Command reads and writes as `application_data` holding
the context-tagged BACnetLightingCommand, operation NONE
(`b"\x09\x00"`) until written. An `octet_string` or any other
datatype is refused with INVALID\_DATA\_TYPE, and a command its operation
can't take with VALUE\_OUT\_OF\_RANGE. The object carries each command
out: a fade or ramp puts its level in Present\_Value at once and moves
Tracking\_Value there over time, with In\_Progress showing FADE\_ACTIVE
or RAMP\_ACTIVE; a step changes the level at once; and with
Blink\_Warn\_Enable TRUE the warn commands hold the level for
Egress\_Time seconds before relinquishing or turning it off.

A Present\_Value or Relinquish\_Default level above 0.0 and below 1.0
is stored as 1.0, and one outside 0.0 to 100.0 is refused with
VALUE\_OUT\_OF\_RANGE, but for Present\_Value's warn values -1.0 (WARN),
-2.0 (WARN\_RELINQUISH) and -3.0 (WARN\_OFF). Tracking\_Value reads the
same level as Present\_Value whenever no fade or ramp is running.

`high_end_trim` and `low_end_trim` (1.0 to 100.0 percent) serve
the trims that bound Tracking\_Value, and with either one
Trim\_Fade\_Time, set in milliseconds (0 to 86400000) by
`trim_fade_time`; without a trim none of the three rows is served.
A value out of range raises BacnetProtocolError (VALUE\_OUT\_OF\_RANGE).
The colour keywords are as on `add_binary_lighting_output`, and the
profile keywords as on `add_analog_input`.

#### BACnetServer.add_binary_lighting_output

```python
def add_binary_lighting_output(
    self,
    instance: int,
    name: str,
    *,
    color_reference: Optional[ObjectIdentifier] = None,
    color_override: Optional[bool] = None,
    override_color_reference: Optional[ObjectIdentifier] = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Add a Binary Lighting Output object.

`color_reference` serves Color\_Reference, the Color or Color
Temperature object the output takes its colour from (instance
4194303 names none). `override_color_reference` adds Color\_Override
and Override\_Color\_Reference, and `color_override=True` turns the
override on; it needs an `override_color_reference`, and both need
a `color_reference` (ValueError otherwise). `color_override=False`
on its own means no override. A reference to another object type raises
BacnetProtocolError (VALUE\_OUT\_OF\_RANGE). All three rows take writes;
`lighting_color` follows them. Profile keywords as on
`add_analog_input`.

#### BACnetServer.add_color

```python
def add_color(
    self,
    instance: int,
    name: str,
    *,
    present_value: Optional[tuple[float, float]] = None,
    default_color: Optional[tuple[float, float]] = None,
    storage_path: Optional[str] = None,
    value_source_tracking: bool = False,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Add a Color object: a CIE 1931 xy colour, D65 white
`(0.3127, 0.3290)` unless `present_value` gives an `(x, y)`
pair, each 0.0 to 1.0 (BacnetProtocolError otherwise). It reads as a
`list` of two `real` values. `value_source_tracking` serves
Value\_Source, the writer of the last Present\_Value or Color\_Command
that set it. Profile keywords as on `add_analog_input`.

At start the server drives the output to `default_color` (D65
unless given), at once, over any `present_value`. `(0.0, 0.0)`
keeps the colour from before the restart instead, kept in
`storage_path` (the file the object keeps Present\_Value in); with
nothing stored it keeps `present_value`, and with neither,
In\_Progress reads NOT\_CONTROLLED until a write drives the output.
A file that doesn't load raises BacnetError.

#### BACnetServer.add_color_temperature

```python
def add_color_temperature(
    self,
    instance: int,
    name: str,
    *,
    present_value: Optional[int] = None,
    min_pres_value: Optional[int] = None,
    max_pres_value: Optional[int] = None,
    default_color_temperature: Optional[int] = None,
    storage_path: Optional[str] = None,
    value_source_tracking: bool = False,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Add a Color Temperature object: a correlated colour temperature in
kelvin, 4000 unless `present_value` says otherwise.
`min_pres_value` and `max_pres_value` narrow the 1000 to 30000
range and come together (ValueError otherwise); `present_value` is
clamped to them. A value out of range raises BacnetProtocolError.
`value_source_tracking` is as on `add_color`.
`default_color_temperature` (4000 unless given, clamped to the
limits) and `storage_path` work as `default_color` and
`storage_path` on `add_color`, with 0 keeping the temperature
from before the restart.

#### BACnetServer.add_channel

```python
def add_channel(
    self,
    instance: int,
    name: str,
    channel_number: int,
    members: Optional[list[tuple[ObjectIdentifier, PropertyIdentifier] | tuple[ObjectIdentifier, PropertyIdentifier, Optional[int]] | DeviceObjectPropertyReference]] = None,
    execution_delay: Optional[list[int]] = None,
    control_groups: Optional[list[int]] = None,
    *,
    allow_group_delay_inhibit: bool = False,
) -> None
```

Add a Channel; a write of its Present\_Value is passed on to `members`.

`channel_number` (0..=65535) is the number a WriteGroup names. Each
member is an `(object, property)` or `(object, property, array_index)` tuple for a property in this device, or a
`DeviceObjectPropertyReference` mapping; one naming this server's
Device is kept in its local form, and one naming another Device is
written there through the server's device bindings
(`add_device_binding` or a heard I-Am). `execution_delay` holds
one delay in milliseconds per member (zeros when omitted),
`control_groups` the WriteGroup groups the Channel is in (`[0]`,
none, when omitted), and `allow_group_delay_inhibit` lets a
WriteGroup that asks for no delays skip them. All of them are
writable over the network too.

A wrong shape or type raises TypeError. An unknown or missing mapping
key or a device that isn't a Device raises ValueError. A channel
number outside unsigned16, or an index (a tuple's or a mapping's), a
delay or a group outside unsigned32, raises OverflowError. A delay
count that differs from the member count or an empty group list
raises BacnetProtocolError with VALUE\_OUT\_OF\_RANGE; more than
1024 members or 64 groups, NO\_SPACE\_TO\_WRITE\_PROPERTY. Nothing is
registered after any of them.

#### BACnetServer.add_life_safety_point

```python
def add_life_safety_point(self, instance: int, name: str) -> None
```

#### BACnetServer.add_life_safety_zone

```python
def add_life_safety_zone(self, instance: int, name: str) -> None
```

#### BACnetServer.add_group

```python
def add_group(
    self,
    instance: int,
    name: str,
    members: Optional[list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]] = None,
) -> None
```

Group whose Present\_Value is rebuilt from `members` on each read.

`members` has the `read_property_multiple` spec shape and the
endpoint `add_group` checks: a member with no properties, or one
reporting a group's Present\_Value, raises ValueError naming its
position and the rule. Any property identifier is taken, those ASHRAE
assigns above 4194303 included. Indexes outside unsigned32 raise
OverflowError.

#### BACnetServer.add_global_group

```python
def add_global_group(
    self,
    instance: int,
    name: str,
    *,
    members: Optional[list[tuple[ObjectIdentifier, PropertyIdentifier] | tuple[ObjectIdentifier, PropertyIdentifier, Optional[int]] | DeviceObjectPropertyReference]] = None,
) -> None
```

Add a Global Group object; `members` seeds Group\_Members, in the
form a typed read of it gives. A member naming this server's Device
is stored in its local form, and a device identifier that isn't a
Device raises ValueError.

#### BACnetServer.add_structured_view

```python
def add_structured_view(self, instance: int, name: str) -> None
```

#### BACnetServer.add_access_door

```python
def add_access_door(
    self,
    instance: int,
    name: str,
    *,
    door_members: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
    alarm_values: Optional[list[int]] = None,
    fault_values: Optional[list[int]] = None,
    masked_alarm_values: Optional[list[int]] = None,
) -> None
```

Add an Access Door object to the server (before starting).

`door_members` sets Door\_Members, the objects that make up the door
(read-only over the network). Each element is an `ObjectIdentifier`
in this device or a `(device, object)` pair of identifiers for one
in another device. A pair whose device isn't a Device object
identifier raises ValueError, and nothing is registered.

`alarm_values`, `fault_values` and `masked_alarm_values` set the
starting Alarm\_Values, Fault\_Values and Masked\_Alarm\_Values, as
BACnetDoorAlarmState numbers other than NORMAL (1 to 8, or 256 to
65535); peers can write all three. Door\_Alarm\_State stays NORMAL or a member of the
alarm or fault values, never a masked state: a simulated value
outside them is refused, and masking the current state returns the
door to NORMAL. An alarm value raises a CHANGE\_OF\_STATE alarm after
Time\_Delay, and a fault value makes Reliability MULTI\_STATE\_FAULT. Any
other number, NORMAL (0) included, raises BacnetProtocolError with
VALUE\_OUT\_OF\_RANGE, and nothing is registered.

#### BACnetServer.add_access_credential

```python
def add_access_credential(
    self,
    instance: int,
    name: str,
    *,
    assigned_access_rights: Optional[list[AssignedAccessRights]] = None,
    authorization_exemptions: Optional[list[int]] = None,
) -> None
```

Configure a credential before starting; invalid input registers nothing.

Assignments default to an empty array. Each mapping needs its reference
and an actual bool enable flag. Missing/unknown keys or a non-Device
device member raise ValueError; incorrect Python types raise TypeError.
Wrong object kinds raise BacnetProtocolError PROPERTY/VALUE\_OUT\_OF\_RANGE,
except instance 4194303, the existing unused marker.

Omitted/None exemptions leave the optional row absent; \[\] serves an
empty row. Integers 0..6 and 64..255 are accepted by the Rust setter.
Bool/non-int elements raise TypeError; values outside u32 raise
OverflowError; reserved values within u32 raise BacnetProtocolError
PROPERTY/VALUE\_OUT\_OF\_RANGE. Value 2 exempts this rights criterion only.

#### BACnetServer.evaluate_access_rights

```python
def evaluate_access_rights(
    self,
    credential: ObjectIdentifier,
    point: ObjectIdentifier,
) -> Awaitable[AccessRightsEvaluation]
```

Inspect the native local rights stage (Clause 12.34.9.2).

Returns a plain mapping. Every key is present; access\_event and rule
are None where inapplicable. A rule index is its one-based position
in the positive/negative array. Unresolved assignments retain their
original one-based indices, order, duplicates and device references;
skipped entries do not compact later indices. Granted may coexist
with unresolved rights; exempt has no unresolved entries.

Granted means this stage passed, not final authorization. Exempt
applies only to this criterion (Clause 12.35.25). The denial event
is evidence, not a recorded Access\_Event. No remote reads, COV/event
reports or physical actions occur. Credential validity, point health,
threat, accompaniment, passback and occupancy remain separate checks.

Requires a started server (RuntimeError otherwise). Missing or wrong
object types raise BacnetProtocolError OBJECT/UNKNOWN\_OBJECT, checking
the credential first. Future unmapped result variants raise RuntimeError.

#### BACnetServer.add_access_point

```python
def add_access_point(
    self,
    instance: int,
    name: str,
    *,
    access_doors: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
    number_of_authentication_policies: Optional[int] = None,
    authentication_policies: Optional[list[tuple[str, AuthenticationPolicy]]] = None,
    supported_authorization_modes: Optional[list[int]] = None,
    priority_for_writing: Optional[int] = None,
) -> None
```

Add an Access Point object to the server (before starting).

`access_doors` sets Access\_Doors (read-only over the network), in the
element forms `add_access_door` takes for `door_members`. A pair
whose device isn't a Device object identifier raises ValueError, and a
reference to anything but an Access Door raises BacnetProtocolError
with VALUE\_OUT\_OF\_RANGE; either way nothing is registered.

`number_of_authentication_policies` (1 when omitted, never 0) and
`priority_for_writing` (16 when omitted, else 1 to 16; outside
0..=255 OverflowError) set Number\_Of\_Authentication\_Policies and
Priority\_For\_Writing, which are read-only over the network. `supported_authorization_modes` lists
the BACnetAuthorizationMode numbers the application carries out, the
values a write of Authorization\_Mode can take: AUTHORIZE (0) alone
when omitted, and AUTHORIZE must be in any list given; proprietary
modes run from 64 to 65535. A value outside those raises
BacnetProtocolError with VALUE\_OUT\_OF\_RANGE. Peers write
Active\_Authentication\_Policy (1 to the policy count) and
Authorization\_Mode (one of the supported modes).

`authentication_policies` sets Authentication\_Policy\_List and
Authentication\_Policy\_Names as `(name, policy)` pairs, both
read-only over the network, and the policy count to their number. A
policy's entries name the Credential Data Inputs and the step each
serves, from 1. An empty list, or more than 256 pairs, raises
BacnetProtocolError with VALUE\_OUT\_OF\_RANGE; an index or timeout
outside 0..=4294967295 raises OverflowError. A policy with no
entries, a reference to anything but a Credential Data Input, or
indexes that don't start at 1 and climb by at most one is kept but
can't be in effect: while it is, Active\_Authentication\_Policy reads 0
until a peer writes a usable policy. While any such policy is listed,
or the active policy is 0, Reliability reads CONFIGURATION\_ERROR and
the point takes no access events. A
`number_of_authentication_policies` given as well is applied after
and resizes both arrays, new policies empty and new names `""`; it
can be at most 256 with the pairs (VALUE\_OUT\_OF\_RANGE above).

#### BACnetServer.add_access_rights

```python
def add_access_rights(
    self,
    instance: int,
    name: str,
    *,
    positive_access_rules: Optional[list[AccessRule]] = None,
    negative_access_rules: Optional[list[AccessRule]] = None,
    enable: bool = True,
    accompaniment: Optional[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]] = None,
    storage_path: Optional[str] = None,
) -> None
```

Add an Access Rights object to the server (before starting).

`positive_access_rules` and `negative_access_rules` set the two
rule arrays as `AccessRule` mappings, and `enable` sets Enable
(`PropertyIdentifier.LOG_ENABLE`, property 133), whose FALSE
disables every rule. Peers can write all three over the network. A
wrong shape or type raises TypeError, an unknown or missing key or a
device that isn't a Device raises ValueError, and a location naming
anything but an Access Point or Access Zone raises BacnetProtocolError
with VALUE\_OUT\_OF\_RANGE, and so does a list of more than 1024 rules
(NO\_SPACE\_TO\_WRITE\_PROPERTY). Nothing is registered after any of
them. The server stores and serves the rules. Applications can call
`evaluate_access_rights` to inspect the local rights stage.

`accompaniment` serves the optional Accompaniment row: the Access
Rights, Access Credential or Access User object that a second
credential, presented with the first, has to match. It takes an
`ObjectIdentifier` in this device or a `(device, object)` pair,
as `door_members` does; instance 4194303 asks for no
accompaniment. Left out, the object has no Accompaniment row and
clients can't add one; once served, clients can write it. A pair
whose device isn't a Device raises ValueError, and any other object
type raises BacnetProtocolError with VALUE\_OUT\_OF\_RANGE. A read
gives the reference back in the form the keyword takes.

With `storage_path`, a rule array, Enable or Accompaniment that a
client writes is kept in that file across restarts, and wins over the
keyword given for it at the next start, which is then checked but not
applied; a saved Accompaniment is served even without the keyword.
To lift a saved requirement, write the no-accompaniment reference
(instance 4194303), which keeps the row; to drop the row, remove the
storage file, which also drops the saved rules and Enable. A
write that cannot be saved is refused with DEVICE /
OPERATIONAL\_PROBLEM and the old value stays. Without it, written
values live in memory only. `storage_path` is a `str`; a
`pathlib.Path` raises TypeError. Give each object a file of its
own: one that holds another object's state, or anything this backend
did not write, makes this call raise BacnetError
(BacnetProtocolError for a saved rule or Accompaniment the object
would refuse).

#### BACnetServer.add_access_user

```python
def add_access_user(
    self,
    instance: int,
    name: str,
    *,
    credentials: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
    members: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
    member_of: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
) -> None
```

Add an Access User object to the server (before starting).

`credentials` sets Credentials, the Access Credential objects the
user holds, and `members` and `member_of` set Members and
Member\_Of, the Access Users one level below and above this one in a
hierarchy of users. All three are read-only over the network and
take the element forms `add_access_door` takes for
`door_members`. A pair whose device isn't a Device object
identifier raises ValueError, and a reference to anything but an
Access Credential in `credentials`, or an Access User in the other
two, raises BacnetProtocolError with VALUE\_OUT\_OF\_RANGE; either way
nothing is registered. A read of a list gives the references back in
the forms the keywords take, or `[]` while it is empty.

#### BACnetServer.add_access_zone

```python
def add_access_zone(
    self,
    instance: int,
    name: str,
    *,
    entry_points: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
    exit_points: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
    alarm_values: Optional[list[int]] = None,
) -> None
```

Add an Access Zone object to the server (before starting).

`entry_points` and `exit_points` set Entry\_Points and Exit\_Points,
the Access Points leading into and out of the zone (read-only over
the network), in the element forms `add_access_door` takes for
`door_members`. A pair whose device isn't a Device object
identifier raises ValueError, and a reference to anything but an
Access Point raises BacnetProtocolError with VALUE\_OUT\_OF\_RANGE;
either way nothing is registered. A read of either list gives the
references back in the forms the keywords take.

`alarm_values` sets the starting Alarm\_Values, the occupancy states
the zone reports with a CHANGE\_OF\_STATE alarm after Time\_Delay, as
BACnetAccessZoneOccupancyState numbers other than NORMAL (1 to 6, or
64 to 65535); peers can write it too. Any other number, NORMAL (0)
included, raises BacnetProtocolError with VALUE\_OUT\_OF\_RANGE and
`first_failed_element_number` naming it, from 1, and nothing is
registered. Left out, the list starts empty.

#### BACnetServer.add_credential_data_input

```python
def add_credential_data_input(
    self,
    instance: int,
    name: str,
    *,
    supported_formats: Optional[list[tuple[int | tuple[int, int | None, int | None], int]]] = None,
) -> None
```

Add a Credential Data Input object to the server (before starting).

`supported_formats` sets Supported\_Formats and
Supported\_Format\_Classes (read-only over the network) as
`(format, format_class)` pairs. A format is a
BACnetAuthenticationFactorType number, or a
`(format_type, vendor_id, vendor_format)` triple whose vendor
members may each be None (absent), as a read gives them; a CUSTOM
format (2) needs both. A format outside the closed production, a CUSTOM
format without its vendor members or a nonzero vendor member on
another format raises BacnetProtocolError with VALUE\_OUT\_OF\_RANGE; a
vendor member above 65535 raises OverflowError. While Out\_Of\_Service is TRUE a client's simulated
Present\_Value must name one of these formats with its class, or be
the UNDEFINED or ERROR factor with class 0.

#### BACnetServer.add_alert_enrollment

```python
def add_alert_enrollment(
    self,
    instance: int,
    name: str,
    initial_source: ObjectIdentifier,
) -> None
```

#### BACnetServer.add_event_enrollment

```python
def add_event_enrollment(
    self,
    instance: int,
    name: str,
    event_type: EventType = ...,
) -> None
```

Add an Event Enrollment (Clause 12.12) to the server (before starting).

Event\_Type follows Event\_Parameters: it reads NONE while they are
unset (the `none` alternative) and the written algorithm's type
after. `event_type` only applies to the legacy octet-string
Event\_Parameters, which name no algorithm: it is the algorithm they
run and the Event\_Type served while they are held.

Object\_Property\_Reference, read-only to peers, reads as one typed
`device_object_property_reference` mapping. It names Analog Input
4194303's Present\_Value while the enrollment has no reference.
Fault\_Parameters reads as the context-tagged `none` choice while no
fault algorithm is set, and writing that clears it; a null written to
it succeeds and changes nothing.

#### BACnetServer.add_elevator_group

```python
def add_elevator_group(
    self,
    instance: int,
    name: str,
    machine_room_id: Optional[ObjectIdentifier] = None,
) -> None
```

Add an Elevator Group; machine\_room\_id must be a Positive Integer Value.

#### BACnetServer.add_escalator

```python
def add_escalator(self, instance: int, name: str) -> None
```

#### BACnetServer.add_lift

```python
def add_lift(self, instance: int, name: str, num_floors: int) -> None
```

#### BACnetServer.add_staging

```python
def add_staging(
    self,
    instance: int,
    name: str,
    present_value: float,
    min_present_value: float,
    units: EngineeringUnits | int,
    priority_for_writing: int,
    stages: list[tuple[float, list[bool], float]],
    target_references: list[ObjectIdentifier],
    stage_names: list[str] | None = None,
) -> None
```

Add a validated local-target Staging object before start().

#### BACnetServer.add_averaging

```python
def add_averaging(
    self,
    instance: int,
    name: str,
    *,
    window_interval: int | None = None,
    window_samples: int | None = None,
) -> None
```

Add an Averaging object before start().

`window_interval` sets Window\_Interval in seconds (900 when omitted)
and `window_samples` sets Window\_Samples (15 when omitted). Peers
can write both, and each write discards the samples. An interval of 0,
or a sample count of 0 or above 1440, raises VALUE\_OUT\_OF\_RANGE.

Object\_Property\_Reference reads as one
`device_object_property_reference` value whose mapping includes all
four keys; while unset its object is Analog Input
4194303's Present\_Value, and writing a reference whose object or
Device is at instance 4194303 unsets it. A null written to it succeeds
and changes nothing.

#### BACnetServer.add_integer_value

```python
def add_integer_value(self, instance: int, name: str) -> None
```

#### BACnetServer.add_positive_integer_value

```python
def add_positive_integer_value(self, instance: int, name: str) -> None
```

#### BACnetServer.add_large_analog_value

```python
def add_large_analog_value(self, instance: int, name: str) -> None
```

#### BACnetServer.add_character_string_value

```python
def add_character_string_value(self, instance: int, name: str) -> None
```

#### BACnetServer.add_octet_string_value

```python
def add_octet_string_value(self, instance: int, name: str) -> None
```

#### BACnetServer.add_bit_string_value

```python
def add_bit_string_value(self, instance: int, name: str) -> None
```

#### BACnetServer.add_accumulator

```python
def add_accumulator(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    scale: float | int | None = None,
    prescale: Optional[tuple[int, int]] = None,
) -> None
```

Add an Accumulator (Clause 12.61) to the server (before starting).

`scale` sets Scale: a `float` is a float scale, sent as a
single-precision REAL that multiplies Present\_Value, and an `int`
an integer scale, the power of ten Present\_Value is multiplied by.
Left out, Scale is the float scale 1.0. `prescale`, a
`(multiplier, modulo_divide)` pair of unsigned32 values, serves the
optional Prescale; left out, the object doesn't serve it and a read
of it is UNKNOWN\_PROPERTY. Both read back as these values (tags
`"scale"` and `"prescale"`). A bool or another type raises
TypeError, an integer outside its type OverflowError, a float that
isn't finite as a REAL or a `prescale` of another length
ValueError; nothing is registered then.

#### BACnetServer.add_pulse_converter

```python
def add_pulse_converter(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
) -> None
```

Add a Pulse Converter (Clause 12.23) to the server (before starting).

Input\_Reference reads as `application_data` holding the
context-tagged reference; while unset it reads as Accumulator
4194303's Present\_Value, and writing a reference to instance 4194303
unsets it. A null written to it succeeds and changes nothing.
While the reference names a missing object, or a property that isn't
an Unsigned or INTEGER, Reliability reads CONFIGURATION\_ERROR and
Status\_Flags FAULT; out of service, Reliability is writable for
simulation. The running server counts each increase of the named
property into Count at least once a second, across an Accumulator's
wrap at Max\_Pres\_Value.

#### BACnetServer.add_file

```python
def add_file(
    self,
    instance: int,
    name: str,
    file_type: str = "application/octet-stream",
) -> None
```

#### BACnetServer.set_file_access_method

```python
def set_file_access_method(self, /, instance: int, access_method: str) -> None
```

Select `"stream"` or `"record"` on a pending built-in File.

This synchronous operation is valid only before `start()`. Select the
access method before loading the corresponding payload; changing modes
does not convert stream data to records or vice versa.

#### BACnetServer.set_file_data

```python
def set_file_data(self, /, instance: int, data: bytes) -> None
```

Copy bytes into a pending stream-access built-in File before `start()`.

#### BACnetServer.get_file_data

```python
def get_file_data(self, /, instance: int) -> bytes
```

Return a fresh bytes copy from a pending stream File before `start()`.

#### BACnetServer.set_file_records

```python
def set_file_records(self, /, instance: int, records: list[bytes]) -> None
```

Copy records into a pending record-access built-in File before `start()`.

#### BACnetServer.get_file_records

```python
def get_file_records(self, /, instance: int) -> list[bytes]
```

Return a fresh list and fresh bytes from a pending record File before `start()`.

#### BACnetServer.set_max_file_size

```python
def set_max_file_size(self, /, instance: int, max_octets: int) -> int
```

Set and return the effective octet growth cap before `start()`.

The built-in File clamp is authoritative. This does not truncate
preloaded content.

#### BACnetServer.set_max_record_count

```python
def set_max_record_count(self, /, instance: int, max_records: int) -> int
```

Set and return the effective record growth cap before `start()`.

The built-in File clamp is authoritative. This does not truncate
preloaded records.

#### BACnetServer.add_bip_network_port

```python
def add_bip_network_port(
    self,
    instance: int,
    name: str,
    *,
    ip_address: str = "0.0.0.0",
    udp_port: int = 47808,
    network_number: int = 0,
    apdu_length: int = 1476,
    subnet_mask: str = "0.0.0.0",
    default_gateway: str = "0.0.0.0",
    dns_servers: Optional[List[str]] = None,
) -> None
```

Add a configured, unbound IPV4 (B/IP) Network Port snapshot.

Every configuration row is read-only and nothing binds a socket.
Raises ValueError for an address that isn't IPv4 (`ip_address`,
`subnet_mask`, `default_gateway` or a DNS server);
BacnetProtocolError for an instance outside 1..255, a
`network_number` of 65535, an `apdu_length` under 50 or an empty
`dns_servers`; OverflowError for a number too large for its type;
RuntimeError once the server has started.

#### BACnetServer.add_bip6_network_port

```python
def add_bip6_network_port(
    self,
    instance: int,
    name: str,
    *,
    ip_address: str = "::",
    udp_port: int = 47808,
    network_number: int = 0,
    apdu_length: int = 1476,
    prefix_length: int = 64,
    default_gateway: str = "::",
    dns_servers: Optional[List[str]] = None,
    multicast_address: str = "ff05::bac0",
    vmac: Optional[bytes] = None,
) -> None
```

Add a configured, unbound IPV6 (B/IPv6) Network Port snapshot.

The B/IPv6 counterpart of `add_bip_network_port`: every row is
read-only and nothing binds a socket. `vmac` is the 3-octet
MAC\_Address until a registered `transport="ipv6"` server publishes
the VMAC it uses; `multicast_address` is the broadcast group.
Raises ValueError for an address that isn't IPv6 or a `vmac` that
isn't 3 octets; BacnetProtocolError for an instance outside 1..255,
a `network_number` of 65535, an `apdu_length` under 50, an empty
`dns_servers`, a `prefix_length` outside 1..128, a
`multicast_address` that isn't multicast or an IPv4-mapped
`ip_address`; OverflowError for a number too large for its type;
RuntimeError once the server has started.

#### BACnetServer.start

```python
def start(self) -> Awaitable[None]
```

Start the server and begin accepting BACnet requests.

#### BACnetServer.stop

```python
def stop(self) -> Awaitable[None]
```

Join admitted work and release the owned transport.

Cancelling the returned Future retains shutdown ownership. Call stop()
again to join it; a cleanup error also retains the owner for retry.
Local mutation is rejected once shutdown starts.

A Notification Forwarder, Notification Class or Audit Log write staged
for a request that stop() cut short is dropped, and stop() waits, with
no limit, until every save those objects queued has run, so storage
holds what they serve. While storage holds it up, a warning naming the
objects still saving is logged after 5 s and every 30 s after that.

#### BACnetServer.local_address

```python
def local_address(self) -> Awaitable[str]
```

Get the retained server instance's last bound address snapshot.

B/IP gives `"ip:port"`. B/IPv6 gives the VMAC peers address the
server by, `"bip6:0x0004D2"` for `device_instance=1234`: an ipv6
server's VMAC is its device instance (Clauses U.1.3, H.7.2), and
startup fails when another node on the link already uses it. MS/TP
gives the station, SC the colon-hex VMAC.

Available during interrupted shutdown; successful stop clears the
instance and subsequent address queries raise RuntimeError.

#### BACnetServer.foreign_registration

```python
def foreign_registration(self) -> Awaitable[ForeignRegistrationStatus]
```

How the foreign-device registration is going (#938), as for
`BACnetClient`. RuntimeError before start and after stop.

#### BACnetServer.read_property

```python
def read_property(
    self,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    array_index: Optional[int] = None,
) -> Awaitable[PropertyValue]
```

Read a property from a local object through the server's
ReadProperty evaluator, the one network reads use.

The result equals what a network `read_property` of the same
property returns: a Group's Present\_Value is rebuilt from its
members, Device instance 4194303 names this server's Device, and the
Device's COV subscription lists are live. Its Device\_Address\_Binding
is a list with one `"address_binding"` element per device binding
the server holds (each `add_device_binding`, and each device whose
I-Am it heard in the last ten minutes): a mapping of
`device_identifier`, `network_number` (0 on this network) and
`mac_address`. An unknown object or
property raises `BacnetProtocolError` with the network error, and a
Group whose member rows exceed `rpm_max_result_elements` raises
`BacnetAbortError` (reason 9, OUT\_OF\_RESOURCES).

#### BACnetServer.write_property_local

```python
def write_property_local(
    self,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    value: PropertyValue,
    priority: Optional[int] = None,
    array_index: Optional[int] = None,
    *,
    source_object: Optional[ObjectIdentifier],
) -> Awaitable[None]
```

Write locally with an explicit source: None selects the server Device.

A source object must exist in the database. Tracked commands require a
concrete server Device; corrections retain the original Device owner.
The value is encoded and decoded as a network WriteProperty's would
be, so any value a network write takes works, including whatever
`read_property` returns, and array-index errors match the network's.

#### BACnetServer.lighting_color

```python
def lighting_color(
    self,
    object_id: ObjectIdentifier,
) -> Awaitable[Optional[tuple[ObjectIdentifier, tuple[float, float] | int]]]
```

The colour the lighting output `object_id` shows now: the
Tracking\_Value of the colour object its Color\_Reference names, or its
Override\_Color\_Reference while Color\_Override is TRUE, so a colour
fade shows as it runs.

Returns `(source, colour)`, the colour object's identifier and an
`(x, y)` pair for a Color object or an `int` in kelvin for a
Color Temperature object, or `None` when the output has no
Color\_Reference, the reference in use names no object (instance
4194303), the server holds no colour object by that identifier, or
that object reads In\_Progress NOT\_CONTROLLED (nothing has driven it).
Raises RuntimeError before `start()`.

#### BACnetServer.set_present_value_local

```python
def set_present_value_local(
    self,
    object_id: ObjectIdentifier,
    value: PropertyValue,
) -> Awaitable[None]
```

Update Present\_Value through the narrow, non-generic application Input authority.

Accepted values are a finite REAL for Analog Input and for a Loop's
control output, logical Enumerated 0/1 (INACTIVE/ACTIVE) for Binary Input,
and Unsigned 1..=Number\_Of\_States for Multi-state Input. Binary values are
BACnet logical values after Polarity, not raw hardware/interface levels.
An object with Out\_Of\_Service set rejects this update to preserve
network simulation ownership. A Life Safety Point or Zone takes an
Enumerated BACnetLifeSafetyState (standard, or 256..=65535) whether or
not Out\_Of\_Service is set, since clients never write its Present\_Value;
Tracking\_Value, Silenced and Operation\_Expected stay as they are. Other
object types are not writable through this method.

#### BACnetServer.report_access_event_local

```python
def report_access_event_local(
    self,
    object_id: ObjectIdentifier,
    event: int,
    tag: int,
    *,
    time: Optional[BACnetTimeStamp] = None,
    credential: Optional[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]] = None,
    authentication_factor: Optional[tuple[int, int, bytes]] = None,
) -> Awaitable[None]
```

Report an access event at an Access Point the server holds.

Access\_Event (`event`, a BACnetAccessEvent number),
Access\_Event\_Tag (`tag`, the access transaction), Access\_Event\_Time
(`time`, the Device clock's when omitted), Access\_Event\_Credential
(`credential`, the no-credential reference when omitted) and
Access\_Event\_Authentication\_Factor (`authentication_factor` as
`(format_type, format_class, value)`, the UNDEFINED factor when
omitted) change together. A credential that isn't an Access
Credential, a factor format outside the closed production, or an
event that is neither a named BACnetAccessEvent nor a proprietary one
from 512 to 65535, raises VALUE\_OUT\_OF\_RANGE. While the point is out
of service, or its Reliability isn't NO\_FAULT\_DETECTED, the event is
refused with WRITE\_ACCESS\_DENIED and nothing changes. An unknown
object raises UNKNOWN\_OBJECT and any object other than an Access
Point OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED. A new time sends the
point's COV report, and a time the point stamps itself always moves,
so events of one transaction each send one.

#### BACnetServer.report_credential_read_local

```python
def report_credential_read_local(
    self,
    object_id: ObjectIdentifier,
    factor: tuple[int, int, bytes],
    *,
    update_time: Optional[BACnetTimeStamp] = None,
) -> Awaitable[None]
```

Report a factor a Credential Data Input's reader has read.

Present\_Value takes `factor`, `(format_type, format_class, value)`, and Update\_Time `update_time`, the Device clock's when
omitted, together. The factor must name one of the reader's
`supported_formats` with its class, or be the UNDEFINED (0) or
ERROR (1) factor with class 0, else VALUE\_OUT\_OF\_RANGE. While the
reader is out of service the read is kept aside, in place of the
reader's earlier one: a client's simulated values stay served, no
COV report goes out, and the return to service serves the latest
read. An unknown object raises UNKNOWN\_OBJECT and any object other
than a Credential Data Input OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED. A
new Update\_Time sends the reader's COV report.

#### BACnetServer.report_door_state_local

```python
def report_door_state_local(
    self,
    object_id: ObjectIdentifier,
    *,
    door_status: Optional[int] = None,
    lock_status: Optional[int] = None,
    door_alarm_state: Optional[int] = None,
) -> Awaitable[None]
```

Report an Access Door's hardware state.

Whichever of Door\_Status, Lock\_Status and Door\_Alarm\_State is given,
as BACnetDoorStatus, BACnetLockStatus and BACnetDoorAlarmState
numbers, changes together; the others keep their values. A number
outside its production, or an alarm state the door's Alarm\_Values,
Fault\_Values and Masked\_Alarm\_Values don't admit, raises
VALUE\_OUT\_OF\_RANGE and nothing changes. While the door is out of
service the values are kept aside, in place of the device's earlier
ones: a client's simulated values stay served, no COV report or event
follows, and the return to service serves the latest values
reported. An unknown object raises UNKNOWN\_OBJECT and any object
other than an Access Door OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED. In
service a new Door\_Alarm\_State sends the door's COV report, and the
door's event algorithm sees it at once.

#### BACnetServer.set_tracking_value_local

```python
def set_tracking_value_local(
    self,
    object_id: ObjectIdentifier,
    value: PropertyValue,
) -> Awaitable[None]
```

Set a Life Safety Point's or Zone's Tracking\_Value, the live state the application derived.

The value is an Enumerated BACnetLifeSafetyState, standard or from
256..=65535: another number raises VALUE\_OUT\_OF\_RANGE and another
datatype INVALID\_DATA\_TYPE. An unknown object raises UNKNOWN\_OBJECT and
any object other than a Life Safety Point or Zone
OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED. Present\_Value, Silenced and
Operation\_Expected stay as they are, so an application that latches
Present\_Value until reset keeps reporting the live state here. While
Out\_Of\_Service is set the value is held for the return to service and
a client's simulated Tracking\_Value stays in place. In service a
SubscribeCOVProperty on Tracking\_Value is notified of the change; a
SubscribeCOV report doesn't carry it.

#### BACnetServer.report_device_reset_local

```python
def report_device_reset_local(self) -> Awaitable[None]
```

Record a reset the application makes for its own reasons: a
DEVICE\_RESET Audit record from this device, with no invoke ID
(#1564). A peer's accepted restart is recorded already. RuntimeError
when the server is not running.

#### BACnetServer.purge_audit_log

```python
def purge_audit_log(self, object_id: ObjectIdentifier) -> Awaitable[None]
```

Purge an Audit Log: clear its records and append a BUFFER\_PURGED status record.

No peer can purge an Audit Log, since its Record\_Count is read-only,
so this is the application's route. The record is appended whether or
not logging is enabled, and also carries LOG\_DISABLED while logging is
off; Total\_Record\_Count keeps counting. The purge reaches storage
before the log serves it. An unknown object
raises UNKNOWN\_OBJECT, any object other than an Audit Log
OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED, and a missing clock or a failed
commit DEVICE / OPERATIONAL\_PROBLEM, leaving the log as it was. A
server that is not running raises RuntimeError.

#### BACnetServer.set_controlled_variable_value_local

```python
def set_controlled_variable_value_local(
    self,
    object_id: ObjectIdentifier,
    value: PropertyValue,
) -> Awaitable[None]
```

Feed a Loop's measured Controlled\_Variable\_Value while the application runs its algorithm.

The value must be a finite REAL: another datatype raises
INVALID\_DATA\_TYPE and NaN or an infinity VALUE\_OUT\_OF\_RANGE. An unknown
object raises UNKNOWN\_OBJECT and any object other than a Loop
OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED. Unlike Present\_Value, the update
is accepted while Out\_Of\_Service is set. A SubscribeCOVProperty on the
property is notified of the change; a SubscribeCOV on the Loop is not,
and its next notification carries the new value. The property stays
read-only over the network.

#### BACnetServer.add_averaging_sample_local

```python
def add_averaging_sample_local(
    self,
    object_id: ObjectIdentifier,
    value: PropertyValue | None,
) -> Awaitable[None]
```

Record one sample for an Averaging object, taken by the application.

The server samples an object holding an Object\_Property\_Reference
itself, every Window\_Interval / Window\_Samples seconds. For an object
without one (its reference reads as the unset form, instance
4194303), the application samples about that often and passes each
result here; on one the server samples, a call is one more attempt and
doesn't move the server's schedule. The value is a BOOLEAN (FALSE and
TRUE count as 0 and 1), Signed, Unsigned, Enumerated or finite REAL;
`None` records an attempt that produced no value, which counts
toward Attempted\_Samples but not Valid\_Samples. Another datatype,
Double included, raises INVALID\_DATA\_TYPE and NaN or an infinity
VALUE\_OUT\_OF\_RANGE, and a refused sample isn't counted. An unknown
object raises UNKNOWN\_OBJECT and any object other than an Averaging
object OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED. Each call fills the next
slot of the Window\_Samples window, dropping the oldest once it is
full, and Minimum\_Value, Maximum\_Value, Average\_Value and the sample
counts change together; with no valid sample in the window they read
positive infinity, negative infinity and NaN. A SubscribeCOVProperty
on one of them is notified when it changes (by the subscription's COV
increment, if it gave one); SubscribeCOV on an Averaging object is
refused.

#### BACnetServer.comm_state

```python
def comm_state(self) -> Awaitable[EnableDisable]
```

Get the DeviceCommunicationControl state.

Returns `EnableDisable.ENABLE` or `EnableDisable.DISABLE_INITIATION`;
the server refuses the deprecated `DISABLE`, so it never reports it.
Raises RuntimeError before start and after stop.

#### BACnetServer.cov_counters

```python
def cov_counters(self) -> Awaitable[CovCounters]
```

Sample COV subscription and notification telemetry; zero at each start.

Every field of the Rust CovCounters under the same name.
subscriptions\_active is a gauge; the rest are running totals.
timed\_changes\_dropped counts timestamped COV-multiple changes lost for
good, in whole or in part, the running signal for a subscriber whose
maximum APDU can't hold one timestamped value.
untimed\_references\_oversized counts each report that left out an
untimestamped reference too large for one notification. Raises
RuntimeError before start and after stop.

#### BACnetServer.event_notification_counters

```python
def event_notification_counters(self) -> Awaitable[EventNotificationCounters]
```

Sample the totals of event notifications the server did not deliver.

Every field of the Rust EventNotificationCounters under the same name,
zero at each start. The recipient-list fields count transitions whose
Notification Class lookup failed closed: the class is missing, its
Recipient\_List can't be read or decoded, or it serves more than 32
destinations. An empty or fully filtered list, DCC and Event\_Enable are
not counted. The next three count matched destinations skipped while
their route was resolved: a Device recipient with no current binding,
a recipient that can't be routed as configured, and a confirmed
recipient at a group address (a broadcast or a multicast one). The
confirmed fields count
notifications to one recipient that found no free invoke ID, were
answered with an Error, Reject or Abort, or drew no acknowledgment
after the last retry. unconfirmed\_send\_failed counts unconfirmed
notifications the transport refused to send, once per destination.
apdu\_too\_large counts notifications not sent to one destination
because they exceed the local APDU size, received\_not\_forwarded
counts received event notifications no Notification Forwarder took,
and forwarding\_cap\_dropped counts destinations dropped because one
notification already had 64 across the forwarders.
Fields are sampled independently. Raises
RuntimeError before start and after stop.

#### BACnetServer.forwarder_save_counters

```python
def forwarder_save_counters(self) -> Awaitable[dict[int, ForwarderSaveCounters]]
```

Sample each Notification Forwarder's save counters, keyed by instance.

The totals belong to the objects: they count from registration, and
a forwarder without `storage_path` stays at zero. Raises
RuntimeError before start and after stop.

#### BACnetServer.dcc_outcome_counters

```python
def dcc_outcome_counters(self) -> Awaitable[DccOutcomeCounters]
```

Sample completed admitted DCC handlers; zero on each new server lifetime.

Accepted means state/timer commit, not response delivery. Independent
samples are not an atomic aggregate. Excludes incomplete cancellation,
duplicates, pre-handler rejection and timer expiry. No trace bridge or
subscriber installation. Raises RuntimeError before start/after stop.

#### BACnetServer.sc_npdu_drop_counts

```python
def sc_npdu_drop_counts(self) -> Awaitable[ScNpduDropCounts]
```

Copy SC ingress counters; requires a started SC owner.

Available while disconnected. Each call returns an independent dictionary;
a new transport starts fresh counters. No network I/O is performed.

#### BACnetServer.request_admission_counters

```python
def request_admission_counters(self) -> Awaitable[RequestAdmissionCounters]
```

Sample counters; RuntimeError before start and after stop.

Admitted totals count registered work, not successful response sends.

### ScHub

BACnet/SC Hub for relaying messages between SC nodes.

Usage:

```python
hub = ScHub("0.0.0.0:47809", "cert.pem", "key.pem", b"\x02\x00\x00\x00\x00\x01",
            ca_cert="ca.pem", device_uuid=provisioned_hub_uuid)
await hub.start()
print(await hub.url())
print(await hub.status())
print(await hub.shutdown_gracefully())  # "graceful" or "forced"
```

The hub also works as an async context manager (starts on entry,
forcefully stops on exit):

```python
async with ScHub(..., device_uuid=provisioned_hub_uuid) as hub:
    ...
```

`ca_cert` must name trusted issuer CA PEM certificates for mutual TLS.
Omitted, None, or empty values raise ValueError at construction. The None
default retains the existing positional layout only; there is no insecure
mode. Invalid/unreadable credentials raise BacnetError from start(), before
binding. Connections require TLS 1.3 and a valid trusted client certificate.

`device_uuid` is required, keyword-only and copied from bytes/bytearray into
owned storage. Missing/None, wrong length or all-zero UUID raises ValueError
before file I/O. The caller provisions this hosting device identity before
deployment and durably reuses the same bytes for its entire lifetime, including
stop/start and fresh objects. No generation/storage or UUID-bit validation is
provided. The hosting port's `vmac` must be six bytes (RuntimeError for wrong
length), neither all zero nor all ff (ValueError). CA presence remains first,
then VMAC validation, then UUID. No certificate-to-identity binding is implied.

Admission and timeout policy is constructor-validated, before bind:
`max_clients`/`max_handshakes` caps (zero/overflowing raise ValueError,
negative values raise OverflowError); `admission_policy` is the static
string `"allow_all"` (default), `"deny_all"` or
`"deny_uuid_replacement"` (unknown strings raise
ValueError, non-strings including callables raise TypeError — no Python
callback can run under the native registry lock). Replacement refusal is a
local security policy before protocol acceptance: it preserves an incumbent
when the same claimed UUID requests its current or another VMAC. Default
mode retains Annex AB replacement; a different-UUID VMAC collision keeps its
standard NAK. UUID equality is not certificate identity proof. Graceful per-peer ack /
close / overall millisecond bounds and handshake TLS / WebSocket-upgrade /
Connect-Request millisecond bounds (out-of-range values raise ValueError).
Optional Hub probes use `probe_scan_interval_ms` / `probe_idle_age_ms` /
`probe_ack_age_ms` / `probe_send_budget_ms` (defaults 30000/60000/5000/5000).
Strictly exceeded ages are checked at scans; ACK age starts at reservation,
not completed send, and is not a hard closure deadline. Delayed ticks are
skipped. Only matching valid ACKs refresh activity. Node keepalive is separate.
`relay_send_budget_ms` bounds each NPDU/opaque unicast, concurrent broadcast
recipient and forwarded BVLC-Result acquisition+send attempt (default 5000),
without timeout-driven retirement, retry or fabricated Result. Probe, control,
cleanup and graceful shutdown stay separate. Unicast counters keep their scope.
These values must be positive whole milliseconds \<=2\*\*63-1 and fit the native
monotonic clock. Broadcast sender/global burst/per-second settings use native
token buckets: bounds 1..=(2\*\*64-1)//1\_000\_000\_000, silent drops, existing counters.
All are constructor-validated before I/O. Negative/over-u64 integers raise
OverflowError, nonintegers TypeError, other invalid bounds ValueError.
`certificate_bindings=None` retains CA-valid admission. A nonempty group
sequence enables mapped-only UUID/VMAC reservations, including offline nodes;
existing admission\_policy can further deny. Shared native validation rejects
overlapping ownership and local Hub VMAC conflicts before PEM I/O/bind.
Inputs are copied; status/repr/errors do not expose certificate contents.
This is installation policy, not end-to-end relayed authentication.
`stop()` is forceful and idempotent; `shutdown_gracefully()` runs the
Disconnect/Ack/close exchange and consumes the hub; dropping the hub
without awaiting close only seals admission and cannot guarantee cleanup.

#### ScHub.__init__

```python
def __init__(
    self,
    listen: str,
    cert: str,
    key: str,
    vmac: bytes,
    ca_cert: Optional[str] = None,
    *,
    device_uuid: Optional[Union[bytes, bytearray]] = None,
    max_clients: int = 256,
    max_handshakes: int = 256,
    admission_policy: Literal["allow_all", "deny_all", "deny_uuid_replacement"] = "allow_all",
    graceful_disconnect_ack_ms: int = 5000,
    graceful_ws_close_ms: int = 5000,
    graceful_overall_ms: int = 15000,
    handshake_tls_ms: int = 10000,
    handshake_websocket_upgrade_ms: int = 10000,
    handshake_connect_request_ms: int = 10000,
    probe_scan_interval_ms: int = 30000,
    probe_idle_age_ms: int = 60000,
    probe_ack_age_ms: int = 5000,
    probe_send_budget_ms: int = 5000,
    broadcast_sender_burst: int = 1024,
    broadcast_sender_per_second: int = 128,
    broadcast_global_burst: int = 4096,
    broadcast_global_per_second: int = 512,
    relay_send_budget_ms: int = 5000,
    certificate_bindings: Sequence[ScHubCertificateBinding] | None = None,
) -> None
```

#### ScHub.start

```python
def start(self) -> Awaitable[None]
```

Start the SC hub.

#### ScHub.stop

```python
def stop(self) -> Awaitable[None]
```

Stop the SC hub (forceful, idempotent).

#### ScHub.shutdown_gracefully

```python
def shutdown_gracefully(self) -> Awaitable[Literal["graceful", "forced"]]
```

Graceful shutdown; consumes the hub (RuntimeError if not started).

#### ScHub.status

```python
def status(self) -> Awaitable[ScHubStatus]
```

Bounded redacted snapshot (RuntimeError before start/after stop).

#### ScHub.__aenter__

```python
def __aenter__(self) -> Awaitable[ScHub]
```

#### ScHub.__aexit__

```python
def __aexit__(
    self,
    _exc_type: Any = None,
    _exc_val: Any = None,
    _exc_tb: Any = None,
) -> Awaitable[None]
```

#### ScHub.address

```python
def address(self) -> Awaitable[Optional[str]]
```

Get the address the hub is listening on (None before start).

#### ScHub.url

```python
def url(self) -> Awaitable[Optional[str]]
```

Get the WebSocket URL of the hub (None before start).

### ReinitializeRequest

A ReinitializeDevice request handed to `on_reinitialize` and
`after_reinitialize` (#1566). The addresses are what the request
claims, not an authenticated identity. Not copyable or picklable.

Properties:

- `state: ReinitializedState`: The requested state.
- `source_mac: bytes`: The immediate link source: the requester, or the last router.
- `source_network: Optional[int]`: The originating network of a routed request, else None.
- `source_address: Optional[bytes]`: The originating MAC of a routed request, else None.
- `invoke_id: int`

### MutationContext

One mutation a `mutation_policy` callable is asked to allow
(#1966): a frozen, redacted copy of the request. It holds the target's
coordinates, never written values or file data. The addresses are what
the request claims, not an authenticated identity. Its repr shows the
service and invoke ID only. Not copyable or picklable.

Properties:

- `service: Literal['write_property', 'write_property_multiple', 'create_object', 'delete_object', 'add_list_element', 'remove_list_element', 'atomic_write_file', 'subscribe_cov', 'subscribe_cov_property', 'subscribe_cov_property_multiple', 'write_group']`: The service. WritePropertyMultiple asks once per element and WriteGroup once per Channel.
- `object_identifier: Optional[ObjectIdentifier]`: The target object (the Channel for WriteGroup, the File for AtomicWriteFile); None for a CreateObject that names only a type and for SubscribeCOVPropertyMultiple.
- `object_type: Optional[ObjectType]`: The target's type, also for a CreateObject by type; None for SubscribeCOVPropertyMultiple.
- `property_identifier: Optional[PropertyIdentifier]`: The property written, listed or subscribed to, else None.
- `array_index: Optional[int]`
- `priority: Optional[int]`: The write priority of a WriteProperty, a WritePropertyMultiple element or a WriteGroup, when there is one.
- `source_mac: bytes`: The immediate link source: the requester, or the last router.
- `source_network: Optional[int]`: The originating network a routed request claims, else None.
- `source_address: Optional[bytes]`: The originating MAC a routed request claims, else None.
- `provenance: Literal["unverified", "relayed_origin", "hub_channel", "direct_peer"]`: How the request arrived: `unverified` for B/IP, B/IPv6, MS/TP and Ethernet, `relayed_origin` through a BACnet/SC hub.
- `principal: Optional[bytes]`: The SHA-256 of the originating peer's verified leaf certificate, for a direct BACnet/SC connection without a routed source. A Python server has no direct SC listener, so this is None.
- `forwarding_peer_principal: Optional[bytes]`: The SHA-256 of the forwarding direct SC peer's leaf certificate, a router included; None, like `principal`, for a Python server.
- `certificate: None`: Always None: a Python server has no direct BACnet/SC listener, and hub-relayed, routed and non-SC requests carry no certificate.
- `forwarding_peer_certificate: None`: Always None, for the reason `certificate` gives.
- `invoke_id: Optional[int]`: A confirmed request's invoke ID; None for WriteGroup.

### ScHubCertificateBinding

Frozen installation group; exact leaf DER SHA-256, with redacted repr.

Inputs are copied. UUID must be nonzero; VMAC and digest lists must be
nonempty and distinct; reserved VMACs are invalid. Raises ValueError.

Properties:

- `uuid: bytes`
- `allowed_vmacs: tuple[bytes, ...]`
- `leaf_sha256: tuple[bytes, ...]`

#### ScHubCertificateBinding.__init__

```python
def __init__(
    self,
    *,
    uuid: bytes,
    allowed_vmacs: Sequence[bytes],
    leaf_sha256: Sequence[bytes],
) -> None
```

### CovCounters

`class CovCounters(TypedDict)`

COV telemetry, zero at each start; fields are sampled one by one, not atomically.

Keys:

- `subscriptions_active: int`
- `subscriptions_created: int`
- `subscriptions_rejected_quota: int`
- `subscriptions_rejected_capacity: int`
- `subscriptions_rejected_indefinite: int`
- `subscriptions_cancelled: int`
- `subscriptions_purged: int`
- `notifications_sent: int`
- `notifications_confirmed: int`
- `notifications_unconfirmed: int`
- `notification_bytes_sent: int`
- `notifications_throttled_fanout: int`
- `notifications_throttled_peer: int`
- `timed_changes_dropped: int`
- `untimed_references_oversized: int`

### CovPolicy

`class CovPolicy(TypedDict, total=False)`

BACnetServer(cov\_policy=...) COV limits, checked at construction; omitted keys keep their defaults.

Keys:

- `max_subscriptions_global: int`
- `max_subscriptions_per_peer: int`
- `reserved_capacity: int`
- `reserved_peers: list[bytes]`
- `reserved_recipients: list[tuple[int | None, bytes]]`
- `allow_indefinite_subscriptions: bool`
- `max_indefinite_per_peer: int`
- `max_notifications_per_event: int`
- `max_notification_bytes_per_event: int`
- `max_confirmed_in_flight_per_peer: int`

### TimeSyncPolicy

`class TimeSyncPolicy(TypedDict, total=False)`

BACnetServer(time\_sync\_policy=...) inbound clock limits, checked at construction; omitted keys keep their defaults.

Keys:

- `enabled: bool`
- `source_restriction: list[tuple[int | None, bytes]] | None`
- `max_step_ms: int | None`
- `per_source_rate: tuple[float, int] | None`
- `global_rate: tuple[float, int] | None`
- `coalesce_window_ms: int`
- `global_coalesce_window_ms: int`
- `max_sources: int`

### ForwarderSaveCounters

`class ForwarderSaveCounters(TypedDict)`

One Notification Forwarder's Subscribed\_Recipients save totals, saturating at 2\*\*64-1.

Keys:

- `failed_saves: int`

### EventNotificationCounters

`class EventNotificationCounters(TypedDict)`

Undelivered event notifications, zero at each start; independent u64 totals, saturating at 2\*\*64-1.

Keys:

- `notification_class_missing: int`
- `recipient_list_unavailable: int`
- `recipient_list_invalid: int`
- `recipient_list_too_long: int`
- `device_recipient_unbound: int`
- `recipient_unroutable: int`
- `confirmed_broadcast_recipient: int`
- `confirmed_no_invoke_id: int`
- `confirmed_rejected: int`
- `confirmed_unanswered: int`
- `unconfirmed_send_failed: int`
- `apdu_too_large: int`
- `received_not_forwarded: int`
- `forwarding_cap_dropped: int`
- `received_not_logged: int`

### DccOutcomeCounters

`class DccOutcomeCounters(TypedDict)`

Independent u64 lifetime totals, saturating at 2\*\*64-1; not an audit log.

Keys:

- `accepted_total: int`
- `policy_denied_total: int`
- `password_failure_total: int`
- `deprecated_denied_total: int`
- `malformed_total: int`

### ScNpduDropCounts

`class ScNpduDropCounts(TypedDict)`

Owned, queue-wide lifetime counters for one started SC transport.

Keys:

- `control_rate_drops: int`
- `fairness_drops: int`
- `full_drops: int`
- `closed_drops: int`

### RequestAdmissionCounters

`class RequestAdmissionCounters(TypedDict)`

Independent counter samples, not a transactionally atomic aggregate.

Keys:

- `recovery_active: int`
- `recovery_admitted_total: int`
- `recovery_overloaded_total: int`
- `confirmed_active: int`
- `confirmed_admitted_total: int`
- `confirmed_overloaded_total: int`
- `confirmed_global_overloaded_total: int`
- `confirmed_peer_overloaded_total: int`
- `confirmed_shutdown_rejected_total: int`
- `unconfirmed_active: int`
- `unconfirmed_admitted_total: int`
- `unconfirmed_overloaded_total: int`
- `unconfirmed_global_overloaded_total: int`
- `unconfirmed_peer_overloaded_total: int`
- `unconfirmed_shutdown_rejected_total: int`
- `abort_active: int`
- `abort_admitted_total: int`
- `confirmed_fallback_dropped_total: int`
- `abort_shutdown_rejected_total: int`

### ScHubOutcomeCounts

`class ScHubOutcomeCounts(TypedDict)`

Per-start saturating outcomes; selected refusals are not delivered NAK counts.

Keys:

- `uuid_replacements: int`
- `vmac_collision_rejections: int`
- `registered_capacity_rejections: int`
- `total_active_accept_drops: int`
- `handshake_accept_drops: int`
- `tls_timeouts: int`
- `websocket_timeouts: int`
- `connect_timeouts: int`
- `unicast_no_target: int`
- `unicast_target_limit: int`
- `unicast_send_timeout: int`
- `unicast_send_error: int`
- `heartbeat_retirements: int`

### ScHubStatus

`class ScHubStatus(TypedDict)`

Bounded hub snapshot: counts and kind labels only (no keys/VMAC maps).

Keys:

- `listening: bool`
- `max_clients: int`
- `max_handshakes: int`
- `client_count: int`
- `handshake_count: int`
- `admin_denied: int`
- `broadcast_sender_exhausted: int`
- `broadcast_global_exhausted: int`
- `outcomes: ScHubOutcomeCounts`

### ReinitializeHandler

```python
ReinitializeHandler = Callable[
    [ReinitializeRequest], Union[None, bool, tuple[ErrorClass, ErrorCode]]
]
```

### AfterReinitialize

```python
AfterReinitialize = Callable[[ReinitializeRequest], Optional[Awaitable[None]]]
```

### MutationHandler

```python
MutationHandler = Callable[[MutationContext], bool]
```

## Router

Owned B/IP routing between numbered networks.

### BACnetRouter

Own B/IP forwarding across one to 255 uniquely numbered ports.

This binding has no local application role, other transport types, or
route administration. It uses native RouterOptions defaults, including
their permissive wire-control policy. The first asynchronous method binds
the object to the running asyncio loop; another loop raises RuntimeError.

Successful awaited close/context exit joins router and transport cleanup.
Cancellation removes only a waiter: await close to observe the barrier.
A cleanup failure raises BacnetRouterCloseError with retained cause details;
subsequent close/start calls re-expose the failure without retry. Restart
is supported only after successful cleanup. Destruction requests best-effort
cleanup on the shared runtime; it is not an awaited cleanup guarantee.

#### BACnetRouter.__init__

```python
def __init__(self, ports: Sequence[BipRouterPort]) -> None
```

#### BACnetRouter.start

```python
def start(self) -> Awaitable[None]
```

Start; duplicate start raises RuntimeError. Native startup errors
retain their exception class and errno. An unsuccessful native start
finishes rollback before reporting its original error.

#### BACnetRouter.close

```python
def close(self) -> Awaitable[None]
```

#### BACnetRouter.__aenter__

```python
def __aenter__(self) -> Awaitable[BACnetRouter]
```

#### BACnetRouter.__aexit__

```python
def __aexit__(
    self,
    exc_type: Any = None,
    exc_value: Any = None,
    traceback: Any = None,
) -> Awaitable[None]
```

Await cleanup and preserve a context-body exception as primary.
A secondary cleanup failure remains observable through later close.

#### BACnetRouter.ports

```python
def ports(self) -> Awaitable[list[RouterPortInfo]]
```

Copy actual addresses in input order; raises RuntimeError before
start or after close begins. Snapshots survive close.

#### BACnetRouter.routes

```python
def routes(self) -> Awaitable[list[RouterRoute]]
```

Copy direct/learned routes; raises RuntimeError before start or
after close begins. No lock or wall-clock timestamp is exposed.

### BipRouterPort

Immutable unopened B/IP port configuration; network\_number is 1..65534.

Port 0 requests an ephemeral UDP port. Mode is normal unless bbmd or
foreign\_device is explicitly supplied; supplying both raises ValueError.
Bbmd's default refusal of foreign-device registrations is unchanged.
Invalid IPv4 addresses raise ValueError; out-of-range integers raise
OverflowError. Construction opens no socket.

Properties:

- `network_number: int`
- `interface: str`
- `port: int`
- `broadcast_address: str`
- `mode: Literal["normal", "bbmd", "foreign_device"]`

#### BipRouterPort.__init__

```python
def __init__(
    self,
    network_number: int,
    *,
    interface: str = "0.0.0.0",
    port: int = 47808,
    broadcast_address: str = "255.255.255.255",
    bbmd: Bbmd | None = None,
    foreign_device: ForeignDevice | None = None,
) -> None
```

### RouterPortInfo

Frozen owned snapshot of an actual started B/IP port.

Properties:

- `index: int`
- `network_number: int`
- `mac: bytes`
- `address: str`

### RouterRoute

Frozen copied route; reachability is effective when captured.

Properties:

- `network_number: int`
- `port_index: int`
- `is_direct: bool`
- `next_hop: bytes | None`
- `reachability: Literal["reachable", "busy", "unreachable"]`

### RouterCleanupFailure

Frozen cause details from the retained native cleanup outcome.

Properties:

- `network_number: int | None`
- `port_index: int | None`
- `kind: Literal["transport", "worker"]`
- `message: str`
- `errno: int | None`
- `error_class: ErrorClass | None`
- `error_code: ErrorCode | None`
- `reason: RejectReason | AbortReason | None`: A remote Reject's `RejectReason` or Abort's `AbortReason`.
- `worker: str | None`

## Shared endpoints

One transport that runs both the client and server roles.

### BipEndpoint

B/IP endpoint: one UDP socket that both initiates and executes.

Two separately constructed objects (`BACnetClient` + `BACnetServer`)
are two connections (two sockets/ports). The endpoint is the
one-transport path: one socket serves both roles.

Admission is lifecycle-lock acquisition, not Future creation. Explicit
second start raises BacnetError; context reentry returns this endpoint.
Close cancels pending connection preparation, then joins earlier admitted
startup and teardown. A later start may restart without old registrations.
Cancelled waiters do not abandon admitted session startup or cleanup;
cancellation can race publication, so always await close for cleanup.
Setup errors are delivered by awaiting start/context entry. BIPv6/Ethernet
have no endpoint owner.

Properties:

- `device_instance: int`
- `vendor_id: int`

#### BipEndpoint.__init__

```python
def __init__(
    self,
    device_instance: int,
    device_name: str = "BACnet Device",
    vendor_id: int = 555,
    interface: str = "0.0.0.0",
    port: int = 0xBAC0,
    broadcast_address: str = "255.255.255.255",
    network_number: int = 0,
    network_port_instance: int = 1,
    max_apdu: int = 1476,
    segmentation: Optional[Segmentation] = None,
    services: Optional[list[int]] = None,
    device_uuid: Optional[Union[bytes, bytearray]] = None,
    queue_capacity: int = 16,
    apdu_timeout_ms: int = 6000,
    apdu_retries: int = 0,
    registered_network_port: Optional[int] = None,
    *,
    read_work_limit: int = 256,
    share_port_by_address: bool = False,
    min_request_interval_ms: int = 0,
    bbmd: Optional[Bbmd] = None,
    foreign_device: Optional[ForeignDevice] = None,
    multicast: Optional[Multicast] = None,
    on_reinitialize: Optional[ReinitializeHandler] = None,
    after_reinitialize: Optional[AfterReinitialize] = None,
    reinit_password: Optional[str] = None,
    device_write_policy: Optional[MutationHandler] = None,
) -> None
```

`on_reinitialize` / `after_reinitialize`: execute
ReinitializeDevice, as `BACnetServer`'s keywords do (#1566);
`reinit_password` (needs `on_reinitialize`, else ValueError; at
most 20 characters, else ValueError) is the password a request must
carry. The callable runs on the session's
one dispatch task, so a slow one stalls every request.
`device_write_policy`: a callable that decides each WriteProperty
to the local Device's Description or installed Audit recipient
(#2033). It gets a `MutationContext` and returns a bool, with the
fail-closed rules of `BACnetServer`'s `mutation_policy` callable:
an exception, a non-bool or interpreter shutdown denies with SERVICES
/ SERVICE\_REQUEST\_DENIED. Without it the endpoint executes no
WriteProperty. It runs with no lock held, but on the session's one
dispatch task, so it too should decide quickly: while it runs, other
requests and `close()` wait for it.
`bbmd` / `foreign_device` (at most one, else ValueError): run
the endpoint as a BBMD, or register it as a foreign device (#938).
`read_work_limit`: result rows one ReadProperty served by the
server role may expand, a Group's member rows included; a read past it
is aborted with OUT\_OF\_RESOURCES. Zero raises ValueError.
`share_port_by_address` (default False): bind the interface address
itself, so endpoints on other addresses of this host can share the
port. Needs an explicit interface and a nonzero port; broadcasts and
unicast are still handled in the order they arrived.
`min_request_interval_ms` (default 0, no pacing; at most 3,600,000,
else ValueError) paces the client role's confirmed requests to each
destination exactly as `BACnetClient`'s keyword does (#1542). A
retry keeps its request's turn; replies, notifications and
unconfirmed requests aren't paced.

#### BipEndpoint.add_analog_input

```python
def add_analog_input(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    present_value: float = 0.0,
) -> None
```

#### BipEndpoint.add_analog_value

```python
def add_analog_value(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
) -> None
```

Optional AV Audit rows: None is absent; priority 'inherit' is present NULL.

#### BipEndpoint.add_binary_input

```python
def add_binary_input(self, instance: int, name: str) -> None
```

#### BipEndpoint.add_binary_value

```python
def add_binary_value(
    self,
    instance: int,
    name: str,
    *,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
) -> None
```

Optional BV Audit rows: None is absent; priority 'inherit' is present NULL.

#### BipEndpoint.add_group

```python
def add_group(
    self,
    instance: int,
    name: str,
    members: Optional[list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]] = None,
) -> None
```

Group whose Present\_Value is rebuilt from `members` on each read.

`members` has the `read_property_multiple` spec shape. A member
with no properties, or one reporting a group's Present\_Value, raises
ValueError naming its position and the rule. Any property identifier
is taken, those ASHRAE assigns above 4194303 included. Indexes
outside unsigned32 raise OverflowError.

#### BipEndpoint.start

```python
def start(self) -> Awaitable[None]
```

Admit one startup; second start while running raises BacnetError.

#### BipEndpoint.close

```python
def close(self) -> Awaitable[None]
```

Join earlier startup and cleanup; idempotent and safe after cancellation.

#### BipEndpoint.__aenter__

```python
def __aenter__(self) -> Awaitable[BipEndpoint]
```

#### BipEndpoint.__aexit__

```python
def __aexit__(
    self,
    _exc_type: Any = None,
    _exc_val: Any = None,
    _exc_tb: Any = None,
) -> Awaitable[None]
```

#### BipEndpoint.client

```python
def client(self) -> Awaitable[EndpointClient]
```

Clone the client role (RuntimeError before start/after close).

#### BipEndpoint.server

```python
def server(self) -> Awaitable[EndpointServer]
```

Clone the server role (RuntimeError before start/after close).

#### BipEndpoint.local_address

```python
def local_address(self) -> Awaitable[str]
```

Active announced IP and actual UDP port; RuntimeError outside active state.

#### BipEndpoint.status

```python
def status(self) -> Awaitable[EndpointStatus]
```

Bounded snapshot (RuntimeError before start/after close).

#### BipEndpoint.foreign_registration

```python
def foreign_registration(self) -> Awaitable[ForeignRegistrationStatus]
```

How the foreign-device registration is going (#938), as for
`BACnetClient`. RuntimeError before start and after close.

#### BipEndpoint.broadcast_i_am

```python
def broadcast_i_am(self) -> Awaitable[None]
```

Broadcast one I-Am consistent with the composed identity.

### ScEndpoint

SC endpoint: one hub connection that both initiates and executes.

`sc_device_uuid` is the single durable lifetime identity for both the
hub dial and DEVICE\_UUID. Lifecycle mirrors `BipEndpoint`.

Properties:

- `device_instance: int`
- `vendor_id: int`

#### ScEndpoint.__init__

```python
def __init__(
    self,
    device_instance: int,
    sc_hub: str,
    sc_vmac: Union[bytes, bytearray],
    sc_ca_cert: str,
    sc_client_cert: str,
    sc_client_key: str,
    *,
    sc_device_uuid: Union[bytes, bytearray],
    device_name: str = "BACnet Device",
    vendor_id: int = 555,
    sc_heartbeat_interval_ms: int = 30000,
    sc_heartbeat_timeout_ms: int = 60000,
    network_number: int = 0,
    network_port_instance: int = 2,
    max_apdu: int = 1476,
    segmentation: Optional[Segmentation] = None,
    services: Optional[list[int]] = None,
    queue_capacity: int = 16,
    read_work_limit: int = 256,
    min_request_interval_ms: int = 0,
    on_reinitialize: Optional[ReinitializeHandler] = None,
    after_reinitialize: Optional[AfterReinitialize] = None,
    reinit_password: Optional[str] = None,
    device_write_policy: Optional[MutationHandler] = None,
) -> None
```

`read_work_limit`, `min_request_interval_ms`,
`on_reinitialize`, `after_reinitialize`, `reinit_password` and
`device_write_policy` are as for `BipEndpoint`.

#### ScEndpoint.add_analog_input

```python
def add_analog_input(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    present_value: float = 0.0,
) -> None
```

#### ScEndpoint.add_analog_value

```python
def add_analog_value(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
) -> None
```

Optional AV Audit rows: None is absent; priority 'inherit' is present NULL.

#### ScEndpoint.add_binary_input

```python
def add_binary_input(self, instance: int, name: str) -> None
```

#### ScEndpoint.add_binary_value

```python
def add_binary_value(
    self,
    instance: int,
    name: str,
    *,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
) -> None
```

Optional BV Audit rows: None is absent; priority 'inherit' is present NULL.

#### ScEndpoint.add_group

```python
def add_group(
    self,
    instance: int,
    name: str,
    members: Optional[list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]] = None,
) -> None
```

Group whose Present\_Value is rebuilt from `members` on each read.

`members` has the `read_property_multiple` spec shape. A member
with no properties, or one reporting a group's Present\_Value, raises
ValueError naming its position and the rule. Any property identifier
is taken, those ASHRAE assigns above 4194303 included. Indexes
outside unsigned32 raise OverflowError.

#### ScEndpoint.start

```python
def start(self) -> Awaitable[None]
```

Admit one hub connection; second start while running raises BacnetError.

#### ScEndpoint.close

```python
def close(self) -> Awaitable[None]
```

Join earlier startup and cleanup; idempotent and safe after cancellation.

#### ScEndpoint.__aenter__

```python
def __aenter__(self) -> Awaitable[ScEndpoint]
```

#### ScEndpoint.__aexit__

```python
def __aexit__(
    self,
    _exc_type: Any = None,
    _exc_val: Any = None,
    _exc_tb: Any = None,
) -> Awaitable[None]
```

#### ScEndpoint.client

```python
def client(self) -> Awaitable[EndpointClient]
```

Clone the client role.

#### ScEndpoint.server

```python
def server(self) -> Awaitable[EndpointServer]
```

Clone the server role.

#### ScEndpoint.local_address

```python
def local_address(self) -> Awaitable[str]
```

VMAC hex for this SC node.

#### ScEndpoint.status

```python
def status(self) -> Awaitable[EndpointStatus]
```

Bounded snapshot (RuntimeError before start/after close).

#### ScEndpoint.broadcast_i_am

```python
def broadcast_i_am(self) -> Awaitable[None]
```

Broadcast one I-Am via the hub relay.

### MstpEndpoint

MS/TP endpoint: one serial owner that both initiates and executes.

Opens a real serial device via `TokioSerialPort` like the current
wrappers. Lifecycle mirrors `BipEndpoint`.

Properties:

- `device_instance: int`
- `vendor_id: int`

#### MstpEndpoint.__init__

```python
def __init__(
    self,
    device_instance: int,
    serial_port: str,
    device_name: str = "BACnet Device",
    vendor_id: int = 555,
    mstp_baud: int = 38400,
    mstp_mac: int = 1,
    mstp_max_master: int = 127,
    mstp_max_info_frames: int = 1,
    max_apdu: int = 480,
    segmentation: Optional[Segmentation] = None,
    services: Optional[list[int]] = None,
    device_uuid: Optional[Union[bytes, bytearray]] = None,
    queue_capacity: int = 16,
    apdu_timeout_ms: int = 6000,
    apdu_retries: int = 0,
    *,
    read_work_limit: int = 256,
    min_request_interval_ms: int = 0,
    mstp_extended_frames: bool = False,
    mstp_echo: bool = False,
    mstp_echo_grace_ms: Optional[int] = None,
    mstp_transmit_latency_pad_ms: Optional[int] = None,
    mstp_usage_timeout_ms: Optional[int] = None,
    mstp_reply_timeout_ms: Optional[int] = None,
    on_reinitialize: Optional[ReinitializeHandler] = None,
    after_reinitialize: Optional[AfterReinitialize] = None,
    reinit_password: Optional[str] = None,
    device_write_policy: Optional[MutationHandler] = None,
) -> None
```

`read_work_limit`, `min_request_interval_ms`,
`on_reinitialize`, `after_reinitialize`, `reinit_password` and
`device_write_policy` are as for `BipEndpoint`. Here
`after_reinitialize` runs once the SimpleACK is queued, before the
token sends it: stopping the endpoint at once can drop the reply, so
wait a short while first.

`max_apdu` must fit the link: 480 by default, or 1476 with
`mstp_extended_frames=True`, which turns on extended frames (Types
32 and 33, NPDUs of up to 1497 octets) as for `BACnetClient`.
Every node on the link must then take extended frames; each port's
transmit queues reserve 768 KiB instead of 272 KiB.

`mstp_echo=True` declares an adapter that returns every octet the
station sends, and `mstp_echo_grace_ms` how long after a frame its
echo may still arrive (None: 40 ms), as for `BACnetClient`. A grace
of 0, or one given without `mstp_echo=True`, raises ValueError here;
a negative grace, or one above 4294967295, raises OverflowError.

`mstp_transmit_latency_pad_ms`, `mstp_usage_timeout_ms` and
`mstp_reply_timeout_ms` are as for `BACnetClient`, but a value
out of range raises ValueError here.

#### MstpEndpoint.add_analog_input

```python
def add_analog_input(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    present_value: float = 0.0,
) -> None
```

#### MstpEndpoint.add_analog_value

```python
def add_analog_value(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
) -> None
```

Optional AV Audit rows: None is absent; priority 'inherit' is present NULL.

#### MstpEndpoint.add_binary_input

```python
def add_binary_input(self, instance: int, name: str) -> None
```

#### MstpEndpoint.add_binary_value

```python
def add_binary_value(
    self,
    instance: int,
    name: str,
    *,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
) -> None
```

Optional BV Audit rows: None is absent; priority 'inherit' is present NULL.

#### MstpEndpoint.add_group

```python
def add_group(
    self,
    instance: int,
    name: str,
    members: Optional[list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]] = None,
) -> None
```

Group whose Present\_Value is rebuilt from `members` on each read.

`members` has the `read_property_multiple` spec shape. A member
with no properties, or one reporting a group's Present\_Value, raises
ValueError naming its position and the rule. Any property identifier
is taken, those ASHRAE assigns above 4194303 included. Indexes
outside unsigned32 raise OverflowError.

#### MstpEndpoint.start

```python
def start(self) -> Awaitable[None]
```

Admit one serial owner; second start while running raises BacnetError.

#### MstpEndpoint.close

```python
def close(self) -> Awaitable[None]
```

Join earlier startup and cleanup; idempotent and safe after cancellation.

#### MstpEndpoint.__aenter__

```python
def __aenter__(self) -> Awaitable[MstpEndpoint]
```

#### MstpEndpoint.__aexit__

```python
def __aexit__(
    self,
    _exc_type: Any = None,
    _exc_val: Any = None,
    _exc_tb: Any = None,
) -> Awaitable[None]
```

#### MstpEndpoint.client

```python
def client(self) -> Awaitable[EndpointClient]
```

Clone the client role.

#### MstpEndpoint.server

```python
def server(self) -> Awaitable[EndpointServer]
```

Clone the server role.

#### MstpEndpoint.local_address

```python
def local_address(self) -> Awaitable[str]
```

Station MAC as a decimal string.

#### MstpEndpoint.status

```python
def status(self) -> Awaitable[EndpointStatus]
```

Bounded snapshot (RuntimeError before start/after close).

#### MstpEndpoint.broadcast_i_am

```python
def broadcast_i_am(self) -> Awaitable[None]
```

Broadcast one I-Am (MS/TP local broadcast).

### EndpointClient

Client role cloned from a running endpoint (no lifecycle).

Initiates `read_property`, `read_range`, `read_property_multiple` and direct B/IP `write_property` through the owner's single transport.
After the owner closes, calls fail closed with `BacnetError`.

#### EndpointClient.read_property

```python
def read_property(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    array_index: Optional[int] = None,
) -> Awaitable[PropertyValue]
```

Read a property through the shared transport.

ACK object/property/index must match. Device/Network Port instance 4194303
requests accept a same-type concrete ACK. Malformed/mismatched ACKs raise
BacnetError; this method returns the property value, not ACK metadata,
shaped as `PropertyValue` describes for read results.

#### EndpointClient.write_property

```python
def write_property(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    value: PropertyValue,
    priority: Optional[int] = None,
    array_index: Optional[int] = None,
    *,
    commandability: Literal["commandable", "noncommandable"],
) -> Awaitable[None]
```

Write to a direct IPv4 B/IP peer through the shared requester.

Commandability is required even without a source Reporter. It controls
Audit filtering, never the wire priority. Omitted commandable priority
means effective 16, including NULL. Invalid literals, priority, raw TLV
framing and APDU size raise ValueError synchronously (out-of-u8 priorities
raise OverflowError). Empty encoded lists are valid. Eligible source
records preserve complete 0–32-byte values and omit longer values.
After source admission, caller cancellation does not cancel the attempt.

#### EndpointClient.read_property_multiple

```python
def read_property_multiple(
    self,
    address: str,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]],
) -> Awaitable[list[ReadAccessResult]]
```

Read 1–64 explicit property occurrences on concrete objects.

Empty lists, ALL/REQUIRED/OPTIONAL and wildcard instances raise ValueError
before address parsing or I/O. Index zero is valid. Complete ordered ACK
correlation permits an inline error to omit its requested index; successful
values must echo it. Byte limits and malformed/mismatched ACKs raise
BacnetError. Returned dictionaries share standalone RPM conversion.

#### EndpointClient.read_range

```python
def read_range(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    array_index: Optional[int] = None,
    range_type: Optional[Literal["position", "sequence", "time"]] = None,
    reference_index: Optional[int] = None,
    reference_seq: Optional[int] = None,
    count: Optional[int] = None,
    *,
    reference_time: ReferenceTime | None = None,
    validation: Literal["strict", "lenient"] = "strict",
) -> Awaitable[ReadRangeResult]
```

Read a range of items from a list or log object.

`range_type` is `"position"`, `"sequence"`, `"time"` or
`None` (all-items). `"time"` reads the `count` items newer than
`reference_time` (older, for a negative count): a naive
`datetime.datetime` in the device's local time, or a
`((year, month, day, day_of_week), (hour, minute, second, hundredths))` pair; an aware datetime raises ValueError, since the
device's zone isn't known here (#1533). Invalid selectors, array index
zero, missing or zero counts and a time that isn't specific raise
ValueError before I/O; a count outside INTEGER16 (-32768..=32767)
raises OverflowError before address parsing or I/O (#1360).
Position/sequence reference zero is valid; omitted references default
to zero.

`validation="strict"` refuses an answer that breaks a ReadRange rule
with BacnetReadRangeViolationError, whose `rule` names it;
`"lenient"` keeps it and lists the rules in `violations` (#1531).
Decode a log's records with `decode_log_records`.

#### EndpointClient.read_log_page

```python
def read_log_page(
    self,
    address: str,
    object_id: ObjectIdentifier,
    cursor: LogCursor = None,
    page_size: int = 100,
) -> Awaitable[LogPage]
```

Read one page of a log's Log\_Buffer from `cursor` (#1530).

`cursor` is `None` or `"oldest"` (the oldest record, found from
Record\_Count and Total\_Record\_Count), `("sequence", n)`,
`("position", n)` or `("time", reference_time)`; lists work
too, so a checkpoint survives JSON. Loop on `page["next"]` until
`page["done"]`, then keep `next` as the checkpoint. One request
is outstanding at a time. A first record past the one asked for sets
`gap`; when the log drops the record asked for, the read starts
over from the oldest. A page answered from before the one asked for,
or none where the counts say records are, raises
BacnetLogNotAdvancingError: such a device's sequence numbers are
inconsistent, so read it from `("position", 1)`. `wrapped` marks
a page that reached the top of the sequence range. A page whose
records don't decode raises BacnetError, dropping the records before
the failing one; an answer that breaks a rule the reader can't
tolerate, such as an echo that doesn't match, raises
BacnetReadRangeViolationError. A non-log object or a `page_size`
outside 1..=32767 raises ValueError before I/O.

#### EndpointClient.service_scope

```python
def service_scope(self) -> dict[str, Any]
```

Narrow scope: initiates reads and direct B/IP `write_property`.

### EndpointServer

Server role cloned from a running endpoint (no lifecycle).

The responder executes `read_property` automatically, WriteProperty to
the local Device through the owner's `device_write_policy`, and
ReinitializeDevice through the owner's `on_reinitialize`, the one
Python callback that runs under a native lock; this handle exposes
liveness plus the MS/TP deferred-reply seam.

#### EndpointServer.is_session_alive

```python
def is_session_alive(self) -> bool
```

True while the owning endpoint is alive and open.

#### EndpointServer.suspend_next_reply

```python
def suspend_next_reply(self) -> None
```

Arm one-shot deferred-reply suspension (MS/TP wiring).

#### EndpointServer.service_scope

```python
def service_scope(self) -> dict[str, Any]
```

Narrow scope: executes `read_property`, plus `write_property`
when the owner has `device_write_policy` and
`reinitialize_device` when it has `on_reinitialize`.

### EndpointStatus

`class EndpointStatus(TypedDict)`

Bounded endpoint snapshot: liveness, identity, transport, leases, policy counts.

Keys:

- `is_running: bool`
- `device_instance: int`
- `vendor_id: int`
- `max_apdu: int`
- `transport: str`
- `local_address: str`
- `active_leases: int`
- `ingress_policy: int`
- `no_server_role: int`
- `no_client_role: int`
- `unclaimed_terminal: int`
- `responder_declined: int`

### ReadRangeResult

`class ReadRangeResult(TypedDict)`

Keys:

- `object_id: ObjectIdentifier`
- `property_id: PropertyIdentifier`
- `array_index: int | None`
- `result_flags: BitString`
- `item_count: int`
- `item_data: bytes`
- `first_sequence_number: int | None`
- `violations: list[str]`

### LogGap

`class LogGap(TypedDict)`

A page whose first record isn't the one asked for.

Keys:

- `expected: int`
- `first: int`
- `skipped: int | None`

### LogPage

`class LogPage(TypedDict)`

One page of `read_log_page`.

`records` are dicts, oldest first, as `decode_log_records` returns
them. `next` is never `None` or `"oldest"`.

Keys:

- `records: list[dict[str, Any]]`
- `first_sequence_number: int | None`
- `result_flags: BitString`
- `gap: LogGap | None`
- `violations: list[str]`
- `next: LogCursor`
- `done: bool`
- `wrapped: bool`

### ReadPropertyResult

`class ReadPropertyResult(TypedDict)`

Keys:

- `property_id: PropertyIdentifier`
- `array_index: int | None`
- `value: PropertyValue | bytes | None`
- `error: tuple[ErrorClass, ErrorCode] | None`

### ReadAccessResult

`class ReadAccessResult(TypedDict)`

Keys:

- `object_id: ObjectIdentifier`
- `results: list[ReadPropertyResult]`

### ReferenceTime

```python
ReferenceTime = Union[
    datetime.datetime,
    tuple[tuple[int, int, int, int], tuple[int, int, int, int]],
]
```

### LogCursor

```python
LogCursor = Union[
    None,
    Literal["oldest"],
    tuple[Literal["sequence"], int],
    tuple[Literal["position"], int],
    tuple[Literal["time"], ReferenceTime],
]
```

## Types and values

Identifiers, property values, structured values and B/IP modes.

### ObjectIdentifier

BACnet Object Identifier (10-bit type + 22-bit instance number).

For u32-representable values, types above 1023 or instances above 4,194,303
raise ValueError. Integers outside u32 raise OverflowError.
Valid proprietary types and the wire wildcard instance are accepted.

`copy.copy`, `copy.deepcopy` and `pickle` (every protocol) rebuild
an equal identifier through this constructor.

Properties:

- `object_type: ObjectType`
- `instance: int`

#### ObjectIdentifier.__init__

```python
def __init__(self, object_type: ObjectType, instance: int) -> None
```

### BACnetTimeStamp

Lossless BACnetTimeStamp CHOICE.

Time components accept their normal BACnet ranges or 255 for unspecified.
Date accepts full years 1900..2154 or 255 for unspecified, months 1..14,
days 1..34, and days-of-week 1..7; each non-year date field also accepts
255 for unspecified. Supplied values are never normalized.

`copy.copy`, `copy.deepcopy` and `pickle` (every protocol) rebuild
an equal timestamp from its encoded CHOICE, so one read from a peer with
a field outside those ranges copies too.

Properties:

- `kind: str`: Selected CHOICE: `sequence_number`, `time`, or `date_time`.
- `value: int | tuple[int, int, int, int] | tuple[tuple[int, int, int, int], tuple[int, int, int, int]]`: Exact selected value, using a full year for the Date tuple (255 when unspecified), as `PropertyValue.date` reads.

#### BACnetTimeStamp.sequence_number

```python
@staticmethod
def sequence_number(value: int) -> BACnetTimeStamp
```

Construct the Sequence Number CHOICE with a value in 0..65535;
another integer raises OverflowError.

#### BACnetTimeStamp.time

```python
@staticmethod
def time(hour: int, minute: int, second: int, hundredths: int) -> BACnetTimeStamp
```

Construct the Time CHOICE.

#### BACnetTimeStamp.date_time

```python
@staticmethod
def date_time(
    date: tuple[int, int, int, int],
    time: tuple[int, int, int, int],
) -> BACnetTimeStamp
```

Construct DateTime from `(full_year, month, day, day_of_week)` and Time tuples.

### BitString

A BACnet bit string (#2080): the wire form plus its bits and names.

`bits` has one bool per bit, bit 0 first. `names` lists the set bits
by their constant names (`"IN_ALARM"`, `"TO_OFFNORMAL"`) when the bit
string's type is known from where it was read, or is None when it isn't.
Two bit strings are equal when their wire forms are; `names` doesn't
take part.

Properties:

- `unused_bits: int`: How many trailing bits of the final octet of `data` are padding.
- `data: bytes`: The octets as carried on the wire.
- `bits: list[bool]`: One bool per bit, bit 0 first.
- `names: list[str] | None`: The set bits by name when the type is known, else None. A set bit without a name (a vendor or reserved position) is left out.

#### BitString.__init__

```python
def __init__(
    self,
    unused_bits: int,
    data: bytes,
    property: Optional[PropertyIdentifier] = None,
) -> None
```

A bit string from its wire form. With `property`, the bits are
named as that property's bit-string type names them (Status\_Flags,
Event\_Enable, Acked\_Transitions, Limit\_Enable,
Protocol\_Services\_Supported, Protocol\_Object\_Types\_Supported).
`unused_bits` is 0 to 7, and 0 when `data` is empty; anything
else raises ValueError.

#### BitString.__len__

```python
def __len__(self) -> int
```

#### BitString.__iter__

```python
def __iter__(self) -> Iterator[bool]
```

The bits as bools, bit 0 first.

#### BitString.__getitem__

```python
def __getitem__(self, index: int) -> bool
```

Bit `index` (negative counts from the last bit); IndexError
past the end.

### PropertyValue

A decoded BACnet property value with tag and Python-native value.

Create values using the static constructors:

```python
PropertyValue.null()
PropertyValue.boolean(True)
PropertyValue.unsigned(42)
PropertyValue.real(3.14)
PropertyValue.character_string("hello")
PropertyValue.object_identifier(oid)
```

A read result (`read_property`, `read_property_multiple`, a
`CovNotification` value and `BACnetServer.read_property`) keeps every
element of the value; only broken framing raises:

- A constructed property with a typed form reads typed. A whole read of
a collection is a `list` of its elements, an indexed read one
element, and a read of a single value that element, tagged with its
production. Where the binding also takes the property as a typed
value, the form is that value's. The properties and forms (see
docs/python-api.md, "Typed constructed values"): Recipient\_List
(`"destination"`: a `Destination` with every key), Port\_Filter
(`"port_permission"`: `(port_id, enabled)`), a Group's
List\_Of\_Group\_Members (`"read_access_specification"`:
`(object_id, [(property_id, array_index), ...])`) and Present\_Value
(`"read_access_result"`: a `ReadAccessResult`), a Command's Action
(`"action_list"`: a list of `ActionCommand` with every key), the
object reference lists and Accompaniment
(`"device_object_reference"`: an `ObjectIdentifier`, or
`(device, object)`), Supported\_Formats
(`"authentication_factor_format"`), Authentication\_Policy\_List
(`"authentication_policy"`: an `AuthenticationPolicy`), Stages
(`"stage_limit_value"`: `(limit, values, deadband)`), Access Rights
rules (`"access_rule"`:
an `AccessRule` with every key), property reference lists and the
single Trend Log Log\_DeviceObjectProperty or Averaging and Event
Enrollment Object\_Property\_Reference
(`"device_object_property_reference"`: a
`DeviceObjectPropertyReference` with every key),
Event\_Algorithm\_Inhibit\_Ref (`"object_property_reference"`: the same
mapping without `device_identifier`), a Global Group's
Present\_Value (`"property_access_result"`),
Audit\_Notification\_Recipient (`"recipient"`: an
`AuditRecipientInput`), Weekly\_Schedule (`"daily_schedule"`),
Exception\_Schedule (`"special_event"`), Effective\_Period
(`"date_range"`), Date\_List (`"calendar_entry"`), timestamps
(`"timestamp"`: a `BACnetTimeStamp`), Active\_COV\_Subscriptions
(`"cov_subscription"`), Device\_Address\_Binding
(`"address_binding"`: `device_identifier`, `network_number` and
`mac_address`), Value\_Source and Value\_Source\_Array
(`"value_source"`), an Accumulator's Scale (`"scale"`: a
`float` or an `int`) and Prescale (`"prescale"`: `(multiplier, modulo_divide)`), Load Control's Requested\_Shed\_Level,
Expected\_Shed\_Level and Actual\_Shed\_Level (`"shed_level"`: a mapping
of `kind`, `percent`, `level` or `amount`, and `value`), an
Access Point's Access\_Event\_Authentication\_Factor and a Credential
Data Input's Present\_Value (`"authentication_factor"`:
`(format_type, format_class, value)`, what
`report_access_event_local` and `report_credential_read_local`
take), and an Event Enrollment's Event\_Parameters
(`"event_parameter"`: a mapping whose `kind` names the algorithm;
see docs/python-api.md). Access\_Event\_Time and Last\_Restore\_Time read
as `"timestamp"`, and Subordinate\_List as a list of
`"device_object_reference"`. Tags elements are `"name_value"`
mappings with `"name": str` and `"value": None | PropertyValue`. Semantic tags
have `None`; valued NULL is `PropertyValue.null()`. Date and Time
are separate primitive values, not a combined pair. Whole Tags reads
are lists, indexed reads one element, and index zero an unsigned count.
Each element keeps its octets, so the value
writes back unchanged. A value that isn't those elements, to the last
octet, follows the rules below.
- Other context-tagged content (a Load Control's shed levels, an Event
Enrollment's Event\_Parameters), or content this type has no form for
(a UCS-4, DBCS or JIS string, bad UTF-8, an ENUMERATED past 32 bits),
is `application_data` holding the octets exactly as served.
- A whole read (no `array_index`) of a property the stack's
classification table marks as an array or list on that object type
(every BACnetARRAY and BACnetLIST of the 2020 object tables) is a
`list` at every length, 0 and 1 included.
- Any other read is the bare value when it holds one element, and a
`list` in wire order when it holds none or several (a date-time is
a date and then a time). An indexed read is one element under these
rules: `Port_Filter[2]` is a port\_permission, `Event_Time_Stamps[1]`
a timestamp.

Two values are equal when they have the same tag and value: numbers
compare as numbers (`real(0.0) == real(-0.0)`, and a NaN equals
nothing), a list item by item, and a typed read by its octets and
element production. Equal values hash alike.

`copy.copy`, `copy.deepcopy` and `pickle` (every protocol) rebuild
an equal value through the constructor its `tag` names, a list from
its items as PropertyValues, and a typed constructed element from the
octets it was read from.

Properties:

- `tag: str`: Type tag: 'null', 'boolean', 'unsigned', 'signed', 'real', 'double', 'octet\_string', 'character\_string', 'bit\_string', 'enumerated', 'date', 'time', 'object\_identifier', 'list', 'application\_data', or one typed constructed element: 'destination', 'port\_permission', 'read\_access\_specification', 'read\_access\_result', 'action\_list', 'device\_object\_reference', 'authentication\_factor\_format', 'stage\_limit\_value', 'access\_rule', 'device\_object\_property\_reference', 'object\_property\_reference', 'property\_access\_result', 'recipient', 'daily\_schedule', 'special\_event', 'calendar\_entry', 'date\_range', 'timestamp', 'cov\_subscription', 'value\_source', 'scale', 'prescale', 'address\_binding', 'shed\_level', 'authentication\_factor', 'event\_parameter'.
- `value: Any`: The Python-native value (int, float, str, bytes, bool, dict, tuple, ObjectIdentifier, BitString, list, or None); `application_data` is `bytes`, and a typed constructed element the form its typed write takes, or the form docs/python-api.md gives it. A date is `(year, month, day, day_of_week)` with the full year, or 255 for an unspecified year, as in every other date the binding reads. A bit string is a `BitString`, named when the value was read from a property of known bit-string type.
- `name: str | None`: The name of an enumerated value read from a property whose enumeration is known (`"HIGH_LIMIT"` for an Event\_State of 3), else None. `value` stays the number. Values built with the constructors carry no property, so their name is None; copies and pickles of a read value keep it.
- `octets: bytes`: The value's octets as a write of it carries them: a typed constructed element's are the octets it was read from, a whole typed collection's its elements' back to back, and any other value its application-tagged encoding. Raises ValueError for a value that doesn't encode.

#### PropertyValue.null

```python
@staticmethod
def null() -> PropertyValue
```

#### PropertyValue.boolean

```python
@staticmethod
def boolean(value: bool) -> PropertyValue
```

#### PropertyValue.unsigned

```python
@staticmethod
def unsigned(value: int) -> PropertyValue
```

#### PropertyValue.signed

```python
@staticmethod
def signed(value: int) -> PropertyValue
```

#### PropertyValue.real

```python
@staticmethod
def real(value: float) -> PropertyValue
```

#### PropertyValue.double

```python
@staticmethod
def double(value: float) -> PropertyValue
```

#### PropertyValue.character_string

```python
@staticmethod
def character_string(value: str) -> PropertyValue
```

#### PropertyValue.octet_string

```python
@staticmethod
def octet_string(value: bytes) -> PropertyValue
```

#### PropertyValue.enumerated

```python
@staticmethod
def enumerated(value: int) -> PropertyValue
```

#### PropertyValue.object_identifier

```python
@staticmethod
def object_identifier(oid: ObjectIdentifier) -> PropertyValue
```

#### PropertyValue.date

```python
@staticmethod
def date(year: int, month: int, day: int, day_of_week: int) -> PropertyValue
```

Create a Date value, in the form `.value` reads it back.

`year` is the full year, 1900..2154, or 255 for unspecified; any
other year raises ValueError. Use 255 for any other unspecified
field.

#### PropertyValue.time

```python
@staticmethod
def time(hour: int, minute: int, second: int, hundredths: int) -> PropertyValue
```

Create a Time value. Use 255 for unspecified fields.

#### PropertyValue.bit_string

```python
@staticmethod
def bit_string(unused_bits: int, data: bytes) -> PropertyValue
```

Create a BitString value.

#### PropertyValue.list

```python
@staticmethod
def list(items: list[PropertyValue]) -> PropertyValue
```

Create a List (array) value from a list of PropertyValue items.
Items that are all elements of one typed constructed collection (from
indexed reads) make that collection, equal to its whole read. Lists
nest at most 32 deep, the decoder's nesting limit; a deeper one
raises ValueError.

#### PropertyValue.application_data

```python
@staticmethod
def application_data(bytes: bytes) -> PropertyValue
```

Create an ApplicationData value from pre-encoded application-layer bytes (e.g. a framed BACnetEventParameter).

### DiscoveredDevice

A device discovered via Who-Is / I-Am.

Properties:

- `object_identifier: ObjectIdentifier`
- `mac_address: bytes`: The MAC the I-Am came from: IPv4 and port on B/IP, the 3-octet VMAC on B/IPv6, the station on MS/TP.
- `max_apdu_length: int`
- `segmentation_supported: Segmentation`
- `vendor_id: int`
- `seconds_since_seen: float`
- `source_network: Optional[int]`
- `source_address: Optional[bytes]`

### CovNotification

A Change-of-Value notification received from a remote device.

Properties:

- `subscriber_process_identifier: int`
- `initiating_device_identifier: ObjectIdentifier`
- `monitored_object_identifier: ObjectIdentifier`
- `time_remaining: int`
- `delivery: str`
- `source_mac: bytes`
- `source_network: Optional[int]`
- `source_address: Optional[bytes]`
- `values: Any`: List of property value change entries: dicts with `property_id`, `array_index` and `value`. `value` is a `PropertyValue` shaped as a read result is, or `bytes` when the octets' framing is broken.

### CovNotificationIterator

Async iterator yielding `CovNotification` objects. One that falls behind
the client's COV channel loses the oldest and adds them to `missed`. A
cancel that lands while a notification is being delivered, as a timed-out
`asyncio.wait_for` does, puts it back: the next `__anext__` returns it
first. Concurrent `__anext__` calls complete one at a time. Read it from
one running event loop (`asyncio.run` is fine): an item on its way to a
read whose loop stops for good holds up later reads, which raise one
`RuntimeWarning` after 5 s and keep waiting. Iteration ends when the
client stops.

Properties:

- `missed: int`: Notifications lost so far: because this iterator fell behind, or dropped on the way to a read, as when its event loop closed first.

#### CovNotificationIterator.__aiter__

```python
def __aiter__(self) -> CovNotificationIterator
```

#### CovNotificationIterator.__anext__

```python
def __anext__(self) -> Awaitable[CovNotification]
```

### IHave

An I-Have this client received (#1850), frozen.

Properties:

- `device_identifier: ObjectIdentifier`
- `object_identifier: ObjectIdentifier`
- `object_name: str`
- `source_mac: bytes`: The link-layer peer it came from: the device, or the router that passed it on.
- `source_network: int | None`: SNET, when a router passed it on from another network.
- `source_address: bytes | None`: SADR, when a router passed it on from another network.

### IHaveIterator

Async iterator over every I-Have from one `i_have_events()` call. The
client never waits for it: one more than 256 behind loses the oldest and
adds them to `missed`. A cancel that lands while an item is being
delivered puts it back: the next `__anext__` returns it first.
Concurrent `__anext__` calls complete one at a time. Read it from one
running event loop (`asyncio.run` is fine): an item on its way to a read
whose loop stops for good holds up later reads, which raise one
`RuntimeWarning` after 5 s and keep waiting. Iteration ends when the
client stops.

Properties:

- `missed: int`: I-Haves lost so far: because this iterator fell behind, or dropped on the way to a read, as when its event loop closed first.

#### IHaveIterator.__aiter__

```python
def __aiter__(self) -> IHaveIterator
```

#### IHaveIterator.__anext__

```python
def __anext__(self) -> Awaitable[IHave]
```

### DiscoveredRouter

A router heard from during `discover_routers()` (#1850), frozen.

Properties:

- `mac: bytes`: The router's MAC on the local network: the next hop to `networks`.
- `networks: list[int]`: Every network it listed, ascending, each once.

### NetworkReport

A network-layer report this client received (#1850), frozen. Fields
that don't apply to its `kind` are None.

Properties:

- `kind: Literal['i_am_router_to_network', 'network_number_is', 'reject_message_to_network', 'other']`
- `source_mac: bytes`: The link-layer peer that sent it: for an I-Am-Router-To-Network, the router to send through.
- `source_network: int | None`: SNET, when a router passed it on from another network.
- `source_address: bytes | None`: SADR, when a router passed it on from another network.
- `networks: list[int] | None`: The networks an I-Am-Router-To-Network lists, in its order.
- `network: int | None`: A Network-Number-Is's network, or the one a refused message was for.
- `configured: bool | None`: Whether a Network-Number-Is sender had its number configured.
- `reason: int | None`: A Reject-Message-To-Network's reason octet.

### NetworkReportIterator

Async iterator over the network reports from one `network_reports()`
call. The client never waits for it: one more than 64 behind loses the
oldest and adds them to `missed`. A cancel that lands while an item is
being delivered puts it back: the next `__anext__` returns it first.
Concurrent `__anext__` calls complete one at a time. Read it from one
running event loop (`asyncio.run` is fine): an item on its way to a read
whose loop stops for good holds up later reads, which raise one
`RuntimeWarning` after 5 s and keep waiting. Iteration ends when the
client stops.

Properties:

- `missed: int`: Reports lost so far: because this iterator fell behind, or dropped on the way to a read, as when its event loop closed first.

#### NetworkReportIterator.__aiter__

```python
def __aiter__(self) -> NetworkReportIterator
```

#### NetworkReportIterator.__anext__

```python
def __anext__(self) -> Awaitable[NetworkReport]
```

### ForeignDevice

A foreign-device registration, for the `foreign_device=` keyword of
`BACnetClient`, `BACnetServer` and `BipEndpoint` (B/IP), and of
`BACnetClient` and `BACnetServer` with `transport="ipv6"` (#1941).

`bbmd` names a B/IP BBMD as `"ip"` or `"ip:port"`, or a B/IPv6 one
as `"ip"`, `"[ip]"` or `"[ip]:port"` (port 47808 by default; an
IPv6 port needs the brackets, since `"2001:db8::10:4780"` is read as a
bare address); the owner's transport must match, else its constructor
raises ValueError. A B/IPv6
BBMD must be a unicast host: an unspecified, multicast, link-local,
IPv4-mapped or IPv4-compatible address, a zone index or port 0 raises
ValueError saying which. `ttl` is how many seconds the BBMD keeps the
entry, 1 to 65535. `renewal` (B/IP only) is the seconds between
registrations, at least 0.1 and less than `ttl`; `None` renews at
half the TTL, which a B/IPv6 foreign device always does (but never more
often than every 30 s), so a B/IPv6 BBMD with `renewal` raises ValueError. Behind NAT,
renew often so the mapping the BBMD's replies come back on stays open,
while the TTL stays long. Invalid values raise ValueError, or
OverflowError for an integer out of range.

The owner registers at start and renews on its own. Its broadcasts go
through the BBMD; on B/IP the first few wait for the first registration
to be answered, and `foreign_registration()` reports how it goes (B/IP
only). A B/IPv6 client needs `ipv6_device_instance`: a B/IPv6 foreign
device uses its instance's VMAC.

Properties:

- `bbmd: str`
- `ttl: int`
- `renewal: Optional[float]`

#### ForeignDevice.__init__

```python
def __init__(self, bbmd: str, ttl: int = 300, renewal: Optional[float] = None) -> None
```

### Bbmd

A BBMD set-up, for the `bbmd=` keyword of `BACnetClient`,
`BACnetServer` and `BipEndpoint` (B/IP only).

`bdt` lists the BBMDs of the other subnets, each `"ip:port"` (its
broadcasts forwarded by unicast) or `("ip:port", "mask")`; this BBMD's
own row is added when missing. `accept_foreign_devices` lets foreign
devices register (default False: every registration is refused).
`management_acl` lists the IPv4 addresses allowed to delete Foreign
Device Table entries (default: none). `bdt_persist_path` names a file
holding a BDT, 10 octets per row, to start with instead of `bdt` when
it loads; a BDT activated through the registered Network Port is saved
there. `alone_on_subnet` says this BBMD is the only device of its
B/IP network on its subnet, as behind a NAT router: forwarded and
distributed broadcasts then go only to its peers and foreign devices,
not to its own subnet (default False). `nat_global_address`,
`"ip"` or `"ip:port"`, runs it behind a NAT router that
port-forwards that public address to it (Annex J.7.5): the BDT must list
the address as this BBMD's own row, every row must be two-hop, and the
Forwarded-NPDUs it builds name that address as their origin.
`nat_private_ranges` lists up to four `"a.b.c.d/n"` ranges to count
as private besides RFC 1918: a foreign device registered from a private
address has its broadcasts sent on from the global address
(`"0.0.0.0/0"` makes that every foreign device). A BDT with rows that
doesn't list the global address, or has a one-hop row, raises
ValueError; an empty one serves foreign devices only. The
registered Network Port serves both as BACnet\_IP\_NAT\_Traversal and
BACnet\_IP\_Global\_Address. Write-BDT from the network is always refused.
Invalid values raise ValueError, or OverflowError for an integer out of
range.

Properties:

- `bdt: list[tuple[str, str]]`
- `accept_foreign_devices: bool`
- `management_acl: list[str]`
- `bdt_persist_path: Optional[pathlib.Path]`
- `alone_on_subnet: bool`
- `nat_global_address: Optional[str]`
- `nat_private_ranges: list[str]`

#### Bbmd.__init__

```python
def __init__(
    self,
    bdt: Optional[Sequence[str | tuple[str, str]]] = None,
    *,
    accept_foreign_devices: bool = False,
    management_acl: Optional[Sequence[str]] = None,
    bdt_persist_path: Optional[str | os.PathLike[str]] = None,
    alone_on_subnet: bool = False,
    nat_global_address: Optional[str] = None,
    nat_private_ranges: Optional[Sequence[str]] = None,
) -> None
```

### Multicast

A B/IP multicast group (Annex J.8), for the `multicast=` keyword of
`BACnetClient`, `BACnetServer` and `BipEndpoint` (B/IP only).

`group` is a class D IPv4 address. The owner's broadcasts go to the
group, and it takes broadcasts only from the group, never a subnet
broadcast; unicast is unchanged. `ttl`, 1 to 255 (default 1), is the
multicast TTL: 1 keeps the group on the local network, so raise it for a
group that spans IP routers, which must forward multicast. `port` is
the group's UDP port, by default the owner's own; several owners on one
host at one address need their own ports and a shared group port.

`group` can't be in 224.0.0.0/24, the block reserved for local
network control.

Multicast needs an explicit `interface` (not `"0.0.0.0"`), a port
for the group (`port=0` needs `Multicast(port=...)`), and can't be
combined with `share_port_by_address=True` or `foreign_device=`:
the owner's constructor raises ValueError. With `bbmd=` the owner is
the BBMD that serves the group (Annex J.8.3): it forwards the group's
broadcasts to its BDT and foreign devices and theirs to the group, and
its own BDT row must have the mask 255.255.255.255, or start raises. A
registered Network Port serves the group as BACnet\_IP\_Multicast\_Address
(0.0.0.0 when off), writable when the server answers ReinitializeDevice
and isn't a BBMD; a group changed that way lasts until the process
exits, and isn't saved. Two `Multicast` values with the same settings
compare and hash equal. Invalid values raise ValueError, or
OverflowError for an integer out of range. Windows multicast is
untested.

Properties:

- `group: str`
- `ttl: int`
- `port: Optional[int]`

#### Multicast.__init__

```python
def __init__(self, group: str, *, ttl: int = 1, port: Optional[int] = None) -> None
```

### UNSPECIFIED

```python
UNSPECIFIED: Literal[255]
```

What an unspecified date or time field holds, the year included: dates
are `(year, month, day, day_of_week)` with the full year, and times
`(hour, minute, second, hundredths)`. `datetime.date(255, ...)` is the
year 255 AD, not a wildcard.

### ActionCommand

`class ActionCommand(TypedDict)`

One write in a Command object's action list (`BACnetActionCommand`).

Unknown keys raise ValueError and wrong types raise TypeError. A
`device_identifier` naming another Device sends that write there as a
confirmed WriteProperty when the server has a binding for the Device;
with none, the command fails when the list runs. A `device_identifier`
that isn't a Device raises ValueError. A read of Action gives each
command in this form, with every key present.

Keys:

- `object_identifier: ObjectIdentifier`
- `property_identifier: PropertyIdentifier`
- `property_value: PropertyValue`
- `property_array_index: NotRequired[int | None]`
- `priority: NotRequired[int | None]`: 1..=16; other values raise BacnetProtocolError (VALUE\_OUT\_OF\_RANGE), and values outside 0..=255 OverflowError.
- `post_delay: NotRequired[int | None]`: Seconds to wait after this write, before the next one or the end.
- `quit_on_failure: NotRequired[bool]`: A failed write with this set stops the rest of the list. Default False.
- `device_identifier: NotRequired[ObjectIdentifier | None]`
- `write_successful: NotRequired[bool]`: A read of Action carries the flag a run sets. add\_command accepts it so a read mapping can be given back, but ignores it: a command starts False.

### DeviceObjectPropertyReference

`class DeviceObjectPropertyReference(TypedDict)`

A property (`BACnetDeviceObjectPropertyReference`), in this device or
the one `device_identifier` names: a Channel or Trend Log Multiple
member, or the property whose value decides when an access rule applies,
such as a Schedule's Present\_Value. A member list also takes an
`(object, property)` or `(object, property, array_index)` tuple for a
property in this device, and stores a member naming the server's own
Device in its local form.

Typed reads include all four keys; absent optional device and array
index fields are `None`. Single reference reads from Trend Log,
Averaging and Event Enrollment use this same mapping. Reserved instance
4194303 remains an explicit ObjectIdentifier, including unset references.

Unknown or missing keys and a `device_identifier` that isn't a Device
raise ValueError, a `property_array_index` outside unsigned32
OverflowError, and wrong types TypeError. The server reads only its own objects, so a Trend Log
Multiple member naming another Device logs a failure instead of a value.
A read of a Channel's, Schedule's or Trend Log Multiple's member list, or
of a Global Group's Group\_Members, gives each reference in this form,
with every key present.

Keys:

- `object_identifier: ObjectIdentifier`
- `property_identifier: PropertyIdentifier`
- `property_array_index: NotRequired[int | None]`: 0..=4294967295; one element of an array property.
- `device_identifier: NotRequired[ObjectIdentifier | None]`

### AccessRule

`class AccessRule(TypedDict)`

One element of an Access Rights rule array (`BACnetAccessRule`).

A missing or `None` `time_range` means the rule applies at any time
(ALWAYS), and a missing or `None` `location` that it covers every
access point (ALL). A `location` is an Access Point or Access Zone, as
an `ObjectIdentifier` in this device or a `(device, object)` pair.
A read of Positive\_Access\_Rules or Negative\_Access\_Rules gives each
rule in this form, with every key present.

Keys:

- `enable: bool`
- `time_range: NotRequired[DeviceObjectPropertyReference | None]`
- `location: NotRequired[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier] | None]`

### AssignedAccessRights

`class AssignedAccessRights(TypedDict)`

One credential assignment; both keys are required.

The reference names Access Rights locally or as a (Device, object) pair.
Instance 4194303 retains the native unused-marker semantics.

Keys:

- `assigned_access_rights: ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]`
- `enable: bool`

### AccessRulePosition

`class AccessRulePosition(TypedDict)`

A rule's original one-based position within its positive or negative array.

Keys:

- `access_rights: ObjectIdentifier`
- `kind: Literal["positive", "negative"]`
- `index: int`

### UnresolvedAccessRights

`class UnresolvedAccessRights(TypedDict)`

An unresolved assignment, retaining its reference and original one-based index.

Keys:

- `index: int`
- `reference: ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]`
- `reason: Literal["remote", "wildcard_device", "not_access_rights", "missing", "unreadable"]`

### AccessRightsEvaluation

`class AccessRightsEvaluation(TypedDict)`

Plain mapping of the native rights stage; every key is always present.

A grant may coexist with unresolved assignments. It is not a final
authorization result. Nullable rule and denial-event evidence use None.
These TypedDicts describe values; no result classes are exported at runtime.

Keys:

- `decision: Literal["granted", "denied", "exempt"]`
- `access_event: int | None`
- `rule: AccessRulePosition | None`
- `unresolved: list[UnresolvedAccessRights]`

### NameValue

`class NameValue(TypedDict)`

One Tags element (`BACnetNameValue`): a tag name, which may not
hold a semicolon, and a primitive value, or `None` (or no `value`
key) for a semantic tag. The `tags` keyword of the object builders
takes a list of these, the form a typed Tags read gives.

Keys:

- `name: str`
- `value: NotRequired[PropertyValue | None]`

### CalendarDate

`class CalendarDate(TypedDict)`

Keys:

- `kind: Literal["date"]`
- `date: BACnetDate`

### CalendarDateRange

`class CalendarDateRange(TypedDict)`

Keys:

- `kind: Literal["date_range"]`
- `start_date: BACnetDate`
- `end_date: BACnetDate`

### CalendarWeekNDay

`class CalendarWeekNDay(TypedDict)`

Keys:

- `kind: Literal["week_n_day"]`
- `month: int`: Each 255 for any.
- `week_of_month: int`
- `day_of_week: int`

### SpecialEvent

`class SpecialEvent(TypedDict)`

One Exception\_Schedule event (`BACnetSpecialEvent`): its period, a
calendar entry or a Calendar's `ObjectIdentifier`, its time-values and
its priority, 1 (highest) to 16.

Keys:

- `period: CalendarEntry | ObjectIdentifier`
- `time_values: list[TimeValue]`
- `priority: int`

### AuditReporterConfiguration

`class AuditReporterConfiguration(TypedDict)`

Owned pre-start target Reporter settings; no Python callbacks.

Keys:

- `instance: int`
- `audit_level: Literal["none", "audit_config", "audit_all"]`
- `auditable_operations: int`
- `issue_confirmed_notifications: bool`
- `monitored_objects: NotRequired[list[ObjectIdentifier | ObjectType | None] | None]`
- `audit_priority_filter: NotRequired[int | None]`
- `maximum_send_delay: NotRequired[int | None]`: Paired Maximum\_Send\_Delay/Send\_Now: None absent; 0..3600 whole seconds.

### AuditRecipientDevice

`class AuditRecipientDevice(TypedDict)`

Audit recipient selected by Device object identifier.

Keys:

- `kind: Literal["device"]`
- `object_identifier: ObjectIdentifier`

### AuditRecipientAddress

`class AuditRecipientAddress(TypedDict)`

Audit recipient selected by BACnet network and MAC address.

Keys:

- `kind: Literal["address"]`
- `network_number: int`
- `mac_address: bytes`

### Destination

`class Destination(TypedDict)`

One Recipient\_List destination (`BACnetDestination`) for
`add_notification_forwarder(recipients=...)`.

Unknown keys, values outside BACnet's range and malformed time tuples
raise ValueError, an integer outside its field's type (unsigned32 for
`process_identifier`, unsigned8 for the bit masks and time fields)
OverflowError, and other wrong types TypeError. A key left out gives a
destination active every day, all day, for every transition, with
unconfirmed notifications. A read of Recipient\_List gives each
destination in this form, with every key present.

Keys:

- `recipient: AuditRecipientInput`
- `process_identifier: int`
- `valid_days: NotRequired[int]`: Bit n is BACnetDaysOfWeek bit n: 1 \<\< 0 is Monday, 1 \<\< 6 Sunday. Default 0x7F.
- `from_time: NotRequired[tuple[int, int, int, int]]`: (hour, minute, second, hundredths); defaults (0, 0, 0, 0) and (23, 59, 59, 99).
- `to_time: NotRequired[tuple[int, int, int, int]]`
- `issue_confirmed_notifications: NotRequired[bool]`
- `transitions: NotRequired[int]`: Bit 0 to-offnormal, bit 1 to-fault, bit 2 to-normal. Default 0x07.

### AuditPropertyReference

`class AuditPropertyReference(TypedDict)`

Keys:

- `property_identifier: PropertyIdentifier`
- `property_array_index: NotRequired[int | None]`

### AuditNotificationInput

`class AuditNotificationInput(TypedDict)`

Keys:

- `source_device: AuditRecipientInput`
- `operation: AuditOperation`
- `target_device: AuditRecipientInput`
- `source_timestamp: NotRequired[BACnetTimeStamp | None]`
- `target_timestamp: NotRequired[BACnetTimeStamp | None]`
- `source_object: NotRequired[ObjectIdentifier | None]`
- `source_comment: NotRequired[str | None]`
- `target_comment: NotRequired[str | None]`
- `invoke_id: NotRequired[int | None]`
- `source_user_id: NotRequired[int | None]`
- `source_user_role: NotRequired[int | None]`
- `target_object: NotRequired[ObjectIdentifier | None]`
- `target_property: NotRequired[AuditPropertyReference | None]`
- `target_priority: NotRequired[int | None]`
- `target_value: NotRequired[bytes | None]`
- `current_value: NotRequired[bytes | None]`
- `result: NotRequired[tuple[ErrorClass, ErrorCode] | None]`

### AuditNotificationRequestInput

`class AuditNotificationRequestInput(TypedDict)`

Keys:

- `notifications: list[AuditNotificationInput]`

### AuditLogQueryByTargetInput

`class AuditLogQueryByTargetInput(TypedDict)`

Keys:

- `kind: Literal["by_target"]`
- `target_device_identifier: ObjectIdentifier`
- `successful_actions_only: Literal[0, 1, 2]`: Corrected BACnetSuccessFilter (RB-02/RB-20): 0 = all, 1 = successes-only, 2 = failures-only. The pre-RB-02 Boolean is rejected with TypeError.
- `target_device_address: NotRequired[AuditRecipientAddress | None]`
- `target_object_identifier: NotRequired[ObjectIdentifier | None]`
- `target_property_identifier: NotRequired[PropertyIdentifier | None]`
- `target_array_index: NotRequired[int | None]`
- `target_priority: NotRequired[int | None]`
- `operations: NotRequired[int | None]`

### AuditLogQueryBySourceInput

`class AuditLogQueryBySourceInput(TypedDict)`

Keys:

- `kind: Literal["by_source"]`
- `source_device_identifier: ObjectIdentifier`
- `successful_actions_only: Literal[0, 1, 2]`: Corrected BACnetSuccessFilter (RB-02/RB-20): 0 = all, 1 = successes-only, 2 = failures-only. The pre-RB-02 Boolean is rejected with TypeError.
- `source_device_address: NotRequired[AuditRecipientAddress | None]`
- `source_object_identifier: NotRequired[ObjectIdentifier | None]`
- `operations: NotRequired[int | None]`

### AuditLogQueryRequestInput

`class AuditLogQueryRequestInput(TypedDict)`

Keys:

- `audit_log: ObjectIdentifier`
- `query_parameters: AuditLogQueryParametersInput`
- `requested_count: int`: Unsigned16 requested count (0..=65535).
- `start_at_sequence_number: NotRequired[int | None]`: Optional corrected Unsigned64 cursor (0..=2\*\*64-1; RB-20).

### AuditPropertyReferenceResult

`class AuditPropertyReferenceResult(TypedDict)`

Keys:

- `property_identifier: PropertyIdentifier`
- `property_array_index: int | None`

### AuditNotification

`class AuditNotification(TypedDict)`

Canonical return mapping for a decoded Audit notification.

Keys:

- `source_timestamp: BACnetTimeStamp | None`
- `target_timestamp: BACnetTimeStamp | None`
- `source_device: AuditRecipientInput`
- `source_object: ObjectIdentifier | None`
- `operation: AuditOperation`
- `source_comment: str | None`
- `target_comment: str | None`
- `invoke_id: int | None`
- `source_user_id: int | None`
- `source_user_role: int | None`
- `target_device: AuditRecipientInput`
- `target_object: ObjectIdentifier | None`
- `target_property: AuditPropertyReferenceResult | None`
- `target_priority: int | None`
- `target_value: bytes | None`
- `current_value: bytes | None`
- `result: tuple[ErrorClass, ErrorCode] | None`

### AtomicReadFileStreamResult

`class AtomicReadFileStreamResult(TypedDict)`

What `atomic_read_file` returns for stream access.

Keys:

- `access_method: Literal["stream"]`
- `end_of_file: bool`
- `start_position: int`
- `data: bytes`

### AtomicReadFileRecordResult

`class AtomicReadFileRecordResult(TypedDict)`

What `atomic_read_file` returns for record access.

Keys:

- `access_method: Literal["record"]`
- `end_of_file: bool`
- `start_record: int`
- `record_count: int`
- `records: list[bytes]`

### AtomicWriteFileStreamResult

`class AtomicWriteFileStreamResult(TypedDict)`

What `atomic_write_file` returns for stream access.

Keys:

- `access_method: Literal["stream"]`
- `start_position: int`

### AtomicWriteFileRecordResult

`class AtomicWriteFileRecordResult(TypedDict)`

What `atomic_write_file` returns for record access.

Keys:

- `access_method: Literal["record"]`
- `start_record: int`

### AuditLogStatusDatum

`class AuditLogStatusDatum(TypedDict)`

Keys:

- `kind: Literal["log_status"]`
- `log_status: BitString`: BACnetLogStatus, named LOG\_DISABLED, BUFFER\_PURGED, LOG\_INTERRUPTED.

### AuditNotificationDatum

`class AuditNotificationDatum(TypedDict)`

Keys:

- `kind: Literal["audit_notification"]`
- `audit_notification: AuditNotification`

### AuditTimeChangeDatum

`class AuditTimeChangeDatum(TypedDict)`

Keys:

- `kind: Literal["time_change"]`
- `time_change: float`

### AuditLogRecord

`class AuditLogRecord(TypedDict)`

Keys:

- `timestamp: BACnetDateTime`
- `datum: AuditLogDatum`

### AuditLogRecordResult

`class AuditLogRecordResult(TypedDict)`

Keys:

- `sequence_number: int`
- `record: AuditLogRecord`

### AuditLogQueryAck

`class AuditLogQueryAck(TypedDict)`

Keys:

- `audit_log: ObjectIdentifier`
- `records: list[AuditLogRecordResult]`
- `no_more_items: bool`

### ForeignRegistrationStatus

`class ForeignRegistrationStatus(TypedDict)`

A foreign device's registration, as `foreign_registration()`
reports it. `registered` is whether a lease is live; the lease starts
at the earliest send a success could answer, so it never outlasts the
BBMD's. `lease_remaining` and `next_attempt_in` are seconds, or None.
`last_bbmd`, `last_result` (the BVLC result code, 0x0030 for a
refusal) and `last_outcome` (`"pending"`, `"accepted"`,
`"refused"`, `"timed_out"`, `"local_error"` or `"cancelled"`)
describe the latest attempt and clear when the owner stops. Those fields
are separate from the live lease: `registered` does not imply the
latest attempt was accepted, and can remain true during a pending,
refused or timed-out renewal. The totals
survive it: `lapses` counts leases that ran out with no newer success,
and `renewal_hints` the BBMD's Distribute-Broadcast-To-Network NAKs,
each of which brings the next attempt forward.

Keys:

- `registered: bool`
- `lease_remaining: Optional[float]`
- `next_attempt_in: Optional[float]`
- `last_bbmd: Optional[str]`
- `last_result: Optional[int]`
- `last_outcome: Optional[str]`
- `attempts: int`
- `accepted: int`
- `refused: int`
- `lapses: int`
- `renewal_hints: int`

### FdtEntry

`class FdtEntry(TypedDict)`

A Foreign Device Table entry, as `read_fdt()` returns it.
`seconds_remaining` is what the BBMD reports, its grace period
included.

Keys:

- `address: str`
- `ttl: int`
- `seconds_remaining: int`

### AuthenticationPolicy

```python
AuthenticationPolicy = tuple[
    list[tuple[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier], int]],
    bool,
    int,
]
```

One Access Point Authentication\_Policy\_List element (`BACnetAuthenticationPolicy`): the `(credential_data_input, index)` entries, each Credential Data Input an `ObjectIdentifier` in this device or a `(device, object)` pair, then whether the order is enforced and the timeout in seconds (0 for none). A read of the list gives each element in this form.

### BACnetDate

```python
BACnetDate = tuple[int, int, int, int]
```

A date `(year, month, day, day_of_week)` with the full year, and a time `(hour, minute, second, hundredths)`; 255 marks an unspecified field.

### BACnetTime

```python
BACnetTime = tuple[int, int, int, int]
```

### TimeValue

```python
TimeValue = tuple[BACnetTime, PropertyValue]
```

One Weekly\_Schedule or exception entry: the time and the value from then.

### CalendarEntry

```python
CalendarEntry = CalendarDate | CalendarDateRange | CalendarWeekNDay
```

One Date\_List entry, or an exception's own period (`BACnetCalendarEntry`).

### AuditRecipientInput

```python
AuditRecipientInput = AuditRecipientDevice | AuditRecipientAddress
```

A read of the Device's Audit\_Notification\_Recipient gives this form.

### AuditLogQueryParametersInput

```python
AuditLogQueryParametersInput = AuditLogQueryByTargetInput | AuditLogQueryBySourceInput
```

### BACnetDateTime

```python
BACnetDateTime = tuple[
    tuple[int, int, int, int],
    tuple[int, int, int, int],
]
```

### AtomicReadFileResult

```python
AtomicReadFileResult = Union[AtomicReadFileStreamResult, AtomicReadFileRecordResult]
```

### AtomicWriteFileResult

```python
AtomicWriteFileResult = Union[AtomicWriteFileStreamResult, AtomicWriteFileRecordResult]
```

### AuditLogDatum

```python
AuditLogDatum = AuditLogStatusDatum | AuditNotificationDatum | AuditTimeChangeDatum
```

## Enumerations

Named BACnet constants. Each also takes vendor or unknown values from their number.

### ObjectType

BACnet object type enumeration (Clause 12).

Standard types 0-64; vendor-proprietary 128-1023.
Use `ObjectType.from_raw(n)` for values not listed here.

65 constants:

`ANALOG_INPUT`, `ANALOG_OUTPUT`, `ANALOG_VALUE`, `BINARY_INPUT`, `BINARY_OUTPUT`, `BINARY_VALUE`, `CALENDAR`, `COMMAND`, `DEVICE`, `EVENT_ENROLLMENT`, `FILE`, `GROUP`, `LOOP`, `MULTI_STATE_INPUT`, `MULTI_STATE_OUTPUT`, `NOTIFICATION_CLASS`, `PROGRAM`, `SCHEDULE`, `AVERAGING`, `MULTI_STATE_VALUE`, `TREND_LOG`, `LIFE_SAFETY_POINT`, `LIFE_SAFETY_ZONE`, `ACCUMULATOR`, `PULSE_CONVERTER`, `EVENT_LOG`, `GLOBAL_GROUP`, `TREND_LOG_MULTIPLE`, `LOAD_CONTROL`, `STRUCTURED_VIEW`, `ACCESS_DOOR`, `TIMER`, `ACCESS_CREDENTIAL`, `ACCESS_POINT`, `ACCESS_RIGHTS`, `ACCESS_USER`, `ACCESS_ZONE`, `CREDENTIAL_DATA_INPUT`, `NETWORK_SECURITY`, `BITSTRING_VALUE`, `CHARACTERSTRING_VALUE`, `DATEPATTERN_VALUE`, `DATE_VALUE`, `DATETIMEPATTERN_VALUE`, `DATETIME_VALUE`, `INTEGER_VALUE`, `LARGE_ANALOG_VALUE`, `OCTETSTRING_VALUE`, `POSITIVE_INTEGER_VALUE`, `TIMEPATTERN_VALUE`, `TIME_VALUE`, `NOTIFICATION_FORWARDER`, `ALERT_ENROLLMENT`, `CHANNEL`, `LIGHTING_OUTPUT`, `BINARY_LIGHTING_OUTPUT`, `NETWORK_PORT`, `ELEVATOR_GROUP`, `ESCALATOR`, `LIFT`, `STAGING`, `AUDIT_REPORTER`, `AUDIT_LOG`, `COLOR`, `COLOR_TEMPERATURE`

Properties:

- `name: str | None`: The constant's name (`"ANALOG_INPUT"`), or None for a value without one.

#### ObjectType.from_raw

```python
@staticmethod
def from_raw(value: int) -> ObjectType
```

#### ObjectType.to_raw

```python
def to_raw(self) -> int
```

### PropertyIdentifier

BACnet property identifier enumeration (Clause 12).

All named properties registered at runtime are listed below as class attributes
matching the BACnet constant names.
Use `PropertyIdentifier.from_raw(n)` for vendor-proprietary values.

486 constants:

`ACKED_TRANSITIONS`, `ACK_REQUIRED`, `ACTION`, `ACTION_TEXT`, `ACTIVE_TEXT`, `ACTIVE_VT_SESSIONS`, `ALARM_VALUE`, `ALARM_VALUES`, `ALL`, `ALL_WRITES_SUCCESSFUL`, `APDU_SEGMENT_TIMEOUT`, `APDU_TIMEOUT`, `APPLICATION_SOFTWARE_VERSION`, `CHANGE_OF_STATE_COUNT`, `CHANGE_OF_STATE_TIME`, `NOTIFICATION_CLASS`, `CONTROLLED_VARIABLE_REFERENCE`, `COV_INCREMENT`, `DATE_LIST`, `DEADBAND`, `DESCRIPTION`, `DEVICE_ADDRESS_BINDING`, `DEVICE_TYPE`, `EFFECTIVE_PERIOD`, `EVENT_ENABLE`, `EVENT_STATE`, `EVENT_TYPE`, `EXCEPTION_SCHEDULE`, `FEEDBACK_VALUE`, `FILE_ACCESS_METHOD`, `FILE_SIZE`, `FILE_TYPE`, `FIRMWARE_REVISION`, `HIGH_LIMIT`, `INACTIVE_TEXT`, `IN_PROCESS`, `LIMIT_ENABLE`, `LIST_OF_GROUP_MEMBERS`, `LIST_OF_OBJECT_PROPERTY_REFERENCES`, `LOCAL_DATE`, `LOCAL_TIME`, `LOCATION`, `LOW_LIMIT`, `MAX_APDU_LENGTH_ACCEPTED`, `MAX_INFO_FRAMES`, `MAX_MASTER`, `MAX_PRES_VALUE`, `MIN_PRES_VALUE`, `MODEL_NAME`, `NOTIFY_TYPE`, `NUMBER_OF_APDU_RETRIES`, `NUMBER_OF_STATES`, `OBJECT_IDENTIFIER`, `OBJECT_LIST`, `OBJECT_NAME`, `OBJECT_PROPERTY_REFERENCE`, `OBJECT_TYPE`, `OUT_OF_SERVICE`, `OUTPUT_UNITS`, `EVENT_PARAMETERS`, `POLARITY`, `PRESENT_VALUE`, `PRIORITY`, `PRIORITY_ARRAY`, `PRIORITY_FOR_WRITING`, `PROCESS_IDENTIFIER`, `PROGRAM_CHANGE`, `PROGRAM_STATE`, `PROTOCOL_OBJECT_TYPES_SUPPORTED`, `PROTOCOL_SERVICES_SUPPORTED`, `PROTOCOL_VERSION`, `RECIPIENT_LIST`, `RELIABILITY`, `RELINQUISH_DEFAULT`, `RESOLUTION`, `SEGMENTATION_SUPPORTED`, `SETPOINT`, `STATE_TEXT`, `STATUS_FLAGS`, `SYSTEM_STATUS`, `TIME_DELAY`, `UNITS`, `UPDATE_INTERVAL`, `UTC_OFFSET`, `VENDOR_IDENTIFIER`, `VENDOR_NAME`, `WEEKLY_SCHEDULE`, `BUFFER_SIZE`, `COV_RESUBSCRIPTION_INTERVAL`, `EVENT_TIME_STAMPS`, `EVENT_MESSAGE_TEXTS`, `EVENT_MESSAGE_TEXTS_CONFIG`, `LOG_BUFFER`, `LOG_DEVICE_OBJECT_PROPERTY`, `LOG_ENABLE`, `LOG_INTERVAL`, `PROTOCOL_REVISION`, `RECORD_COUNT`, `START_TIME`, `STOP_TIME`, `STOP_WHEN_FULL`, `TOTAL_RECORD_COUNT`, `ACTIVE_COV_SUBSCRIPTIONS`, `DATABASE_REVISION`, `MAINTENANCE_REQUIRED`, `MEMBER_OF`, `MODE`, `SILENCED`, `TRACKING_VALUE`, `ZONE_MEMBERS`, `LIFE_SAFETY_ALARM_VALUES`, `MAX_SEGMENTS_ACCEPTED`, `PROFILE_NAME`, `SCHEDULE_DEFAULT`, `LOGGING_OBJECT`, `LOGGING_TYPE`, `ALIGN_INTERVALS`, `INTERVAL_OFFSET`, `LAST_RESTART_REASON`, `TIME_OF_DEVICE_RESTART`, `TIME_SYNCHRONIZATION_INTERVAL`, `UTC_TIME_SYNCHRONIZATION_RECIPIENTS`, `NODE_SUBTYPE`, `NODE_TYPE`, `STRUCTURED_OBJECT_LIST`, `SUBORDINATE_LIST`, `ACTUAL_SHED_LEVEL`, `REQUESTED_SHED_LEVEL`, `PROPERTY_LIST`, `EVENT_DETECTION_ENABLE`, `EVENT_ALGORITHM_INHIBIT`, `EVENT_ALGORITHM_INHIBIT_REF`, `TIME_DELAY_NORMAL`, `RELIABILITY_EVALUATION_INHIBIT`, `FAULT_TYPE`, `CHANNEL_NUMBER`, `CONTROL_GROUPS`, `EXECUTION_DELAY`, `NETWORK_NUMBER`, `NETWORK_TYPE`, `MAC_ADDRESS`, `COMMAND_TIME_ARRAY`, `CURRENT_COMMAND_PRIORITY`, `LAST_COMMAND_TIME`, `VALUE_SOURCE`, `VALUE_SOURCE_ARRAY`, `BACNET_IPV6_MODE`, `TAGS`, `PRESENT_STAGE`, `STAGES`, `STAGE_NAMES`, `AUDIT_LEVEL`, `DEVICE_UUID`, `ARCHIVE`, `BIAS`, `CONTROLLED_VARIABLE_UNITS`, `CONTROLLED_VARIABLE_VALUE`, `DAYLIGHT_SAVINGS_STATUS`, `DERIVATIVE_CONSTANT`, `DERIVATIVE_CONSTANT_UNITS`, `DESCRIPTION_OF_HALT`, `ELAPSED_ACTIVE_TIME`, `ERROR_LIMIT`, `FAULT_VALUES`, `INSTANCE_OF`, `INTEGRAL_CONSTANT`, `INTEGRAL_CONSTANT_UNITS`, `ISSUE_CONFIRMED_NOTIFICATIONS`, `MANIPULATED_VARIABLE_REFERENCE`, `MAXIMUM_OUTPUT`, `MINIMUM_OFF_TIME`, `MINIMUM_ON_TIME`, `MINIMUM_OUTPUT`, `MODIFICATION_DATE`, `OPTIONAL`, `PROGRAM_LOCATION`, `PROPORTIONAL_CONSTANT`, `PROPORTIONAL_CONSTANT_UNITS`, `READ_ONLY`, `REASON_FOR_HALT`, `REQUIRED`, `SETPOINT_REFERENCE`, `TIME_OF_ACTIVE_TIME_RESET`, `TIME_OF_STATE_COUNT_RESET`, `TIME_SYNCHRONIZATION_RECIPIENTS`, `VT_CLASSES_SUPPORTED`, `ATTEMPTED_SAMPLES`, `AVERAGE_VALUE`, `CLIENT_COV_INCREMENT`, `MAXIMUM_VALUE`, `MINIMUM_VALUE`, `NOTIFICATION_THRESHOLD`, `RECORDS_SINCE_NOTIFICATION`, `VALID_SAMPLES`, `WINDOW_INTERVAL`, `WINDOW_SAMPLES`, `MAXIMUM_VALUE_TIMESTAMP`, `MINIMUM_VALUE_TIMESTAMP`, `VARIANCE_VALUE`, `BACKUP_FAILURE_TIMEOUT`, `CONFIGURATION_FILES`, `DIRECT_READING`, `LAST_RESTORE_TIME`, `OPERATION_EXPECTED`, `SETTING`, `AUTO_SLAVE_DISCOVERY`, `MANUAL_SLAVE_ADDRESS_BINDING`, `SLAVE_ADDRESS_BINDING`, `SLAVE_PROXY_ENABLE`, `LAST_NOTIFY_RECORD`, `ACCEPTED_MODES`, `ADJUST_VALUE`, `COUNT`, `COUNT_BEFORE_CHANGE`, `COUNT_CHANGE_TIME`, `COV_PERIOD`, `INPUT_REFERENCE`, `LIMIT_MONITORING_INTERVAL`, `LOGGING_RECORD`, `PRESCALE`, `PULSE_RATE`, `SCALE`, `SCALE_FACTOR`, `UPDATE_TIME`, `VALUE_BEFORE_CHANGE`, `VALUE_SET`, `VALUE_CHANGE_TIME`, `RESTART_NOTIFICATION_RECIPIENTS`, `TRIGGER`, `SUBORDINATE_ANNOTATIONS`, `DUTY_WINDOW`, `EXPECTED_SHED_LEVEL`, `FULL_DUTY_BASELINE`, `SHED_DURATION`, `SHED_LEVEL_DESCRIPTIONS`, `SHED_LEVELS`, `STATE_DESCRIPTION`, `DOOR_ALARM_STATE`, `DOOR_EXTENDED_PULSE_TIME`, `DOOR_MEMBERS`, `DOOR_OPEN_TOO_LONG_TIME`, `DOOR_PULSE_TIME`, `DOOR_STATUS`, `DOOR_UNLOCK_DELAY_TIME`, `LOCK_STATUS`, `MASKED_ALARM_VALUES`, `SECURED_STATUS`, `ABSENTEE_LIMIT`, `ACCESS_ALARM_EVENTS`, `ACCESS_DOORS`, `ACCESS_EVENT`, `ACCESS_EVENT_AUTHENTICATION_FACTOR`, `ACCESS_EVENT_CREDENTIAL`, `ACCESS_EVENT_TIME`, `ACCESS_TRANSACTION_EVENTS`, `ACCOMPANIMENT`, `ACCOMPANIMENT_TIME`, `ACTIVATION_TIME`, `ACTIVE_AUTHENTICATION_POLICY`, `ASSIGNED_ACCESS_RIGHTS`, `AUTHENTICATION_FACTORS`, `AUTHENTICATION_POLICY_LIST`, `AUTHENTICATION_POLICY_NAMES`, `AUTHENTICATION_STATUS`, `AUTHORIZATION_MODE`, `BELONGS_TO`, `CREDENTIAL_DISABLE`, `CREDENTIAL_STATUS`, `CREDENTIALS`, `CREDENTIALS_IN_ZONE`, `DAYS_REMAINING`, `ENTRY_POINTS`, `EXIT_POINTS`, `EXPIRATION_TIME`, `EXTENDED_TIME_ENABLE`, `FAILED_ATTEMPT_EVENTS`, `FAILED_ATTEMPTS`, `FAILED_ATTEMPTS_TIME`, `LAST_ACCESS_EVENT`, `LAST_ACCESS_POINT`, `LAST_CREDENTIAL_ADDED`, `LAST_CREDENTIAL_ADDED_TIME`, `LAST_CREDENTIAL_REMOVED`, `LAST_CREDENTIAL_REMOVED_TIME`, `LAST_USE_TIME`, `LOCKOUT`, `LOCKOUT_RELINQUISH_TIME`, `MAX_FAILED_ATTEMPTS`, `MEMBERS`, `MUSTER_POINT`, `NEGATIVE_ACCESS_RULES`, `NUMBER_OF_AUTHENTICATION_POLICIES`, `OCCUPANCY_COUNT`, `OCCUPANCY_COUNT_ADJUST`, `OCCUPANCY_COUNT_ENABLE`, `OCCUPANCY_LOWER_LIMIT`, `OCCUPANCY_LOWER_LIMIT_ENFORCED`, `OCCUPANCY_STATE`, `OCCUPANCY_UPPER_LIMIT`, `OCCUPANCY_UPPER_LIMIT_ENFORCED`, `PASSBACK_MODE`, `PASSBACK_TIMEOUT`, `POSITIVE_ACCESS_RULES`, `REASON_FOR_DISABLE`, `SUPPORTED_FORMATS`, `SUPPORTED_FORMAT_CLASSES`, `THREAT_AUTHORITY`, `THREAT_LEVEL`, `TRACE_FLAG`, `TRANSACTION_NOTIFICATION_CLASS`, `USER_EXTERNAL_IDENTIFIER`, `USER_INFORMATION_REFERENCE`, `USER_NAME`, `USER_TYPE`, `USES_REMAINING`, `ZONE_FROM`, `ZONE_TO`, `ACCESS_EVENT_TAG`, `GLOBAL_IDENTIFIER`, `VERIFICATION_TIME`, `BASE_DEVICE_SECURITY_POLICY`, `DISTRIBUTION_KEY_REVISION`, `DO_NOT_HIDE`, `KEY_SETS`, `LAST_KEY_SERVER`, `NETWORK_ACCESS_SECURITY_POLICIES`, `PACKET_REORDER_TIME`, `SECURITY_PDU_TIMEOUT`, `SECURITY_TIME_WINDOW`, `SUPPORTED_SECURITY_ALGORITHMS`, `UPDATE_KEY_SET_TIMEOUT`, `BACKUP_AND_RESTORE_STATE`, `BACKUP_PREPARATION_TIME`, `RESTORE_COMPLETION_TIME`, `RESTORE_PREPARATION_TIME`, `BIT_MASK`, `BIT_TEXT`, `IS_UTC`, `GROUP_MEMBERS`, `GROUP_MEMBER_NAMES`, `MEMBER_STATUS_FLAGS`, `REQUESTED_UPDATE_INTERVAL`, `COVU_PERIOD`, `COVU_RECIPIENTS`, `FAULT_PARAMETERS`, `LOCAL_FORWARDING_ONLY`, `PROCESS_IDENTIFIER_FILTER`, `SUBSCRIBED_RECIPIENTS`, `PORT_FILTER`, `AUTHORIZATION_EXEMPTIONS`, `ALLOW_GROUP_DELAY_INHIBIT`, `LAST_PRIORITY`, `WRITE_STATUS`, `SERIAL_NUMBER`, `BLINK_WARN_ENABLE`, `DEFAULT_FADE_TIME`, `DEFAULT_RAMP_RATE`, `DEFAULT_STEP_INCREMENT`, `EGRESS_TIME`, `IN_PROGRESS`, `INSTANTANEOUS_POWER`, `LIGHTING_COMMAND`, `LIGHTING_COMMAND_DEFAULT_PRIORITY`, `MAX_ACTUAL_VALUE`, `MIN_ACTUAL_VALUE`, `POWER`, `TRANSITION`, `EGRESS_ACTIVE`, `INTERFACE_VALUE`, `FAULT_HIGH_LIMIT`, `FAULT_LOW_LIMIT`, `LOW_DIFF_LIMIT`, `STRIKE_COUNT`, `TIME_OF_STRIKE_COUNT_RESET`, `DEFAULT_TIMEOUT`, `INITIAL_TIMEOUT`, `LAST_STATE_CHANGE`, `STATE_CHANGE_VALUES`, `TIMER_RUNNING`, `TIMER_STATE`, `APDU_LENGTH`, `IP_ADDRESS`, `IP_DEFAULT_GATEWAY`, `IP_DHCP_ENABLE`, `IP_DHCP_LEASE_TIME`, `IP_DHCP_LEASE_TIME_REMAINING`, `IP_DHCP_SERVER`, `IP_DNS_SERVER`, `BACNET_IP_GLOBAL_ADDRESS`, `BACNET_IP_MODE`, `BACNET_IP_MULTICAST_ADDRESS`, `BACNET_IP_NAT_TRAVERSAL`, `IP_SUBNET_MASK`, `BACNET_IP_UDP_PORT`, `BBMD_ACCEPT_FD_REGISTRATIONS`, `BBMD_BROADCAST_DISTRIBUTION_TABLE`, `BBMD_FOREIGN_DEVICE_TABLE`, `CHANGES_PENDING`, `COMMAND_NP`, `FD_BBMD_ADDRESS`, `FD_SUBSCRIPTION_LIFETIME`, `LINK_SPEED`, `LINK_SPEEDS`, `LINK_SPEED_AUTONEGOTIATE`, `NETWORK_INTERFACE_NAME`, `NETWORK_NUMBER_QUALITY`, `ROUTING_TABLE`, `VIRTUAL_MAC_ADDRESS_TABLE`, `IPV6_ADDRESS`, `IPV6_PREFIX_LENGTH`, `BACNET_IPV6_UDP_PORT`, `IPV6_DEFAULT_GATEWAY`, `BACNET_IPV6_MULTICAST_ADDRESS`, `IPV6_DNS_SERVER`, `IPV6_AUTO_ADDRESSING_ENABLE`, `IPV6_DHCP_LEASE_TIME`, `IPV6_DHCP_LEASE_TIME_REMAINING`, `IPV6_DHCP_SERVER`, `IPV6_ZONE_INDEX`, `ASSIGNED_LANDING_CALLS`, `CAR_ASSIGNED_DIRECTION`, `CAR_DOOR_COMMAND`, `CAR_DOOR_STATUS`, `CAR_DOOR_TEXT`, `CAR_DOOR_ZONE`, `CAR_DRIVE_STATUS`, `CAR_LOAD`, `CAR_LOAD_UNITS`, `CAR_MODE`, `CAR_MOVING_DIRECTION`, `CAR_POSITION`, `ELEVATOR_GROUP`, `ENERGY_METER`, `ENERGY_METER_REF`, `ESCALATOR_MODE`, `FAULT_SIGNALS`, `FLOOR_TEXT`, `GROUP_ID`, `GROUP_MODE`, `HIGHER_DECK`, `INSTALLATION_ID`, `LANDING_CALLS`, `LANDING_CALL_CONTROL`, `LANDING_DOOR_STATUS`, `LOWER_DECK`, `MACHINE_ROOM_ID`, `MAKING_CAR_CALL`, `NEXT_STOPPING_FLOOR`, `OPERATION_DIRECTION`, `PASSENGER_ALARM`, `POWER_MODE`, `REGISTERED_CAR_CALL`, `ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS`, `PROTOCOL_LEVEL`, `REFERENCE_PORT`, `DEPLOYED_PROFILE_LOCATION`, `PROFILE_LOCATION`, `SUBORDINATE_NODE_TYPES`, `SUBORDINATE_TAGS`, `SUBORDINATE_RELATIONSHIPS`, `DEFAULT_SUBORDINATE_RELATIONSHIP`, `REPRESENTS`, `DEFAULT_PRESENT_VALUE`, `TARGET_REFERENCES`, `AUDIT_SOURCE_REPORTER`, `AUDIT_NOTIFICATION_RECIPIENT`, `AUDIT_PRIORITY_FILTER`, `AUDITABLE_OPERATIONS`, `DELETE_ON_FORWARD`, `MAXIMUM_SEND_DELAY`, `MONITORED_OBJECTS`, `SEND_NOW`, `FLOOR_NUMBER`, `ADDITIONAL_REFERENCE_PORTS`, `CERTIFICATE_SIGNING_REQUEST_FILE`, `COMMAND_VALIDATION_RESULT`, `ISSUER_CERTIFICATE_FILES`, `COLOR_OVERRIDE`, `COLOR_REFERENCE`, `DEFAULT_COLOR`, `DEFAULT_COLOR_TEMPERATURE`, `OVERRIDE_COLOR_REFERENCE`, `COLOR_COMMAND`, `HIGH_END_TRIM`, `LOW_END_TRIM`, `TRIM_FADE_TIME`

Properties:

- `name: str | None`: The constant's name (`"ACKED_TRANSITIONS"`), or None for a value without one.

#### PropertyIdentifier.from_raw

```python
@staticmethod
def from_raw(value: int) -> PropertyIdentifier
```

#### PropertyIdentifier.to_raw

```python
def to_raw(self) -> int
```

### ErrorClass

`class ErrorClass(enum.IntEnum)`

BACnet error class enumeration (Clause 18).

An `enum.IntEnum` (#1816): members are ints, compare equal to their
numbers and hash like them, so an int works wherever a member does. A
value without a name (a proprietary or unknown one) is a pseudo-member:
`ErrorClass(1000)` and `ErrorClass.from_raw(1000)` both give one, equal to
1000, with `name` None and repr `ErrorClass(1000)`. A number outside
0 to 65535 raises ValueError.

8 constants:

`DEVICE`, `OBJECT`, `PROPERTY`, `RESOURCES`, `SECURITY`, `SERVICES`, `VT`, `COMMUNICATION`

#### ErrorClass.from_raw

```python
@classmethod
def from_raw(cls, value: int) -> ErrorClass
```

The member for `value`; a pseudo-member when it has no name.

#### ErrorClass.to_raw

```python
def to_raw(self) -> int
```

The raw integer value.

### ErrorCode

`class ErrorCode(enum.IntEnum)`

BACnet error code enumeration (Clause 18).

An `enum.IntEnum` (#1816): members are ints, compare equal to their
numbers and hash like them, so an int works wherever a member does. A
value without a name (a proprietary or unknown one) is a pseudo-member:
`ErrorCode(1000)` and `ErrorCode.from_raw(1000)` both give one, equal to
1000, with `name` None and repr `ErrorCode(1000)`. A number outside
0 to 65535 raises ValueError.

151 constants:

`OTHER`, `AUTHENTICATION_FAILED`, `CONFIGURATION_IN_PROGRESS`, `DEVICE_BUSY`, `DYNAMIC_CREATION_NOT_SUPPORTED`, `FILE_ACCESS_DENIED`, `INCOMPATIBLE_SECURITY_LEVELS`, `INCONSISTENT_PARAMETERS`, `INCONSISTENT_SELECTION_CRITERION`, `INVALID_DATA_TYPE`, `INVALID_FILE_ACCESS_METHOD`, `INVALID_FILE_START_POSITION`, `INVALID_OPERATOR_NAME`, `INVALID_PARAMETER_DATA_TYPE`, `INVALID_TIME_STAMP`, `KEY_GENERATION_ERROR`, `MISSING_REQUIRED_PARAMETER`, `NO_OBJECTS_OF_SPECIFIED_TYPE`, `NO_SPACE_FOR_OBJECT`, `NO_SPACE_TO_ADD_LIST_ELEMENT`, `NO_SPACE_TO_WRITE_PROPERTY`, `NO_VT_SESSIONS_AVAILABLE`, `PROPERTY_IS_NOT_A_LIST`, `OBJECT_DELETION_NOT_PERMITTED`, `OBJECT_IDENTIFIER_ALREADY_EXISTS`, `OPERATIONAL_PROBLEM`, `PASSWORD_FAILURE`, `READ_ACCESS_DENIED`, `SECURITY_NOT_SUPPORTED`, `SERVICE_REQUEST_DENIED`, `TIMEOUT`, `UNKNOWN_OBJECT`, `UNKNOWN_PROPERTY`, `UNKNOWN_VT_CLASS`, `UNKNOWN_VT_SESSION`, `UNSUPPORTED_OBJECT_TYPE`, `VALUE_OUT_OF_RANGE`, `VT_SESSION_ALREADY_CLOSED`, `VT_SESSION_TERMINATION_FAILURE`, `WRITE_ACCESS_DENIED`, `CHARACTER_SET_NOT_SUPPORTED`, `INVALID_ARRAY_INDEX`, `COV_SUBSCRIPTION_FAILED`, `NOT_COV_PROPERTY`, `OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`, `INVALID_CONFIGURATION_DATA`, `DATATYPE_NOT_SUPPORTED`, `DUPLICATE_NAME`, `DUPLICATE_OBJECT_ID`, `PROPERTY_IS_NOT_AN_ARRAY`, `ABORT_BUFFER_OVERFLOW`, `ABORT_INVALID_APDU_IN_THIS_STATE`, `ABORT_PREEMPTED_BY_HIGHER_PRIORITY_TASK`, `ABORT_SEGMENTATION_NOT_SUPPORTED`, `ABORT_PROPRIETARY`, `ABORT_OTHER`, `INVALID_TAG`, `NETWORK_DOWN`, `REJECT_BUFFER_OVERFLOW`, `REJECT_INCONSISTENT_PARAMETERS`, `REJECT_INVALID_PARAMETER_DATA_TYPE`, `REJECT_INVALID_TAG`, `REJECT_MISSING_REQUIRED_PARAMETER`, `REJECT_PARAMETER_OUT_OF_RANGE`, `REJECT_TOO_MANY_ARGUMENTS`, `REJECT_UNDEFINED_ENUMERATION`, `REJECT_UNRECOGNIZED_SERVICE`, `REJECT_PROPRIETARY`, `REJECT_OTHER`, `UNKNOWN_DEVICE`, `UNKNOWN_ROUTE`, `VALUE_NOT_INITIALIZED`, `INVALID_EVENT_STATE`, `NO_ALARM_CONFIGURED`, `LOG_BUFFER_FULL`, `LOGGED_VALUE_PURGED`, `NO_PROPERTY_SPECIFIED`, `NOT_CONFIGURED_FOR_TRIGGERED_LOGGING`, `UNKNOWN_SUBSCRIPTION`, `PARAMETER_OUT_OF_RANGE`, `LIST_ELEMENT_NOT_FOUND`, `BUSY`, `COMMUNICATION_DISABLED`, `SUCCESS`, `ACCESS_DENIED`, `BAD_DESTINATION_ADDRESS`, `BAD_DESTINATION_DEVICE_ID`, `BAD_SIGNATURE`, `BAD_SOURCE_ADDRESS`, `BAD_TIMESTAMP`, `CANNOT_USE_KEY`, `CANNOT_VERIFY_MESSAGE_ID`, `CORRECT_KEY_REVISION`, `DESTINATION_DEVICE_ID_REQUIRED`, `DUPLICATE_MESSAGE`, `ENCRYPTION_NOT_CONFIGURED`, `ENCRYPTION_REQUIRED`, `INCORRECT_KEY`, `INVALID_KEY_DATA`, `KEY_UPDATE_IN_PROGRESS`, `MALFORMED_MESSAGE`, `NOT_KEY_SERVER`, `SECURITY_NOT_CONFIGURED`, `SOURCE_SECURITY_REQUIRED`, `TOO_MANY_KEYS`, `UNKNOWN_AUTHENTICATION_TYPE`, `UNKNOWN_KEY`, `UNKNOWN_KEY_REVISION`, `UNKNOWN_SOURCE_MESSAGE`, `NOT_ROUTER_TO_DNET`, `ROUTER_BUSY`, `UNKNOWN_NETWORK_MESSAGE`, `MESSAGE_TOO_LONG`, `SECURITY_ERROR`, `ADDRESSING_ERROR`, `WRITE_BDT_FAILED`, `READ_BDT_FAILED`, `REGISTER_FOREIGN_DEVICE_FAILED`, `READ_FDT_FAILED`, `DELETE_FDT_ENTRY_FAILED`, `DISTRIBUTE_BROADCAST_FAILED`, `UNKNOWN_FILE_SIZE`, `ABORT_APDU_TOO_LONG`, `ABORT_APPLICATION_EXCEEDED_REPLY_TIME`, `ABORT_OUT_OF_RESOURCES`, `ABORT_TSM_TIMEOUT`, `ABORT_WINDOW_SIZE_OUT_OF_RANGE`, `FILE_FULL`, `INCONSISTENT_CONFIGURATION`, `INCONSISTENT_OBJECT_TYPE`, `INTERNAL_ERROR`, `NOT_CONFIGURED`, `OUT_OF_MEMORY`, `VALUE_TOO_LONG`, `ABORT_INSUFFICIENT_SECURITY`, `ABORT_SECURITY_ERROR`, `DUPLICATE_ENTRY`, `INVALID_VALUE_IN_THIS_STATE`, `INVALID_OPERATION_IN_THIS_STATE`, `LIST_ITEM_NOT_NUMBERED`, `LIST_ITEM_NOT_TIMESTAMPED`, `INVALID_DATA_ENCODING`, `BVLC_FUNCTION_UNKNOWN`, `BVLC_PROPRIETARY_FUNCTION_UNKNOWN`, `HEADER_ENCODING_ERROR`, `HEADER_NOT_UNDERSTOOD`, `MESSAGE_INCOMPLETE`, `NOT_A_BACNET_SC_HUB`, `PAYLOAD_EXPECTED`, `UNEXPECTED_DATA`, `NODE_DUPLICATE_VMAC`

#### ErrorCode.from_raw

```python
@classmethod
def from_raw(cls, value: int) -> ErrorCode
```

The member for `value`; a pseudo-member when it has no name.

#### ErrorCode.to_raw

```python
def to_raw(self) -> int
```

The raw integer value.

### RejectReason

`class RejectReason(enum.IntEnum)`

BACnet reject reason enumeration (Clause 18.9).

An `enum.IntEnum` (#1816): members are ints, compare equal to their
numbers and hash like them, so an int works wherever a member does. A
value without a name (a proprietary or unknown one) is a pseudo-member:
`RejectReason(200)` and `RejectReason.from_raw(200)` both give one, equal to
200, with `name` None and repr `RejectReason(200)`. A number outside
0 to 255 raises ValueError.

11 constants:

`OTHER`, `BUFFER_OVERFLOW`, `INCONSISTENT_PARAMETERS`, `INVALID_PARAMETER_DATA_TYPE`, `INVALID_TAG`, `MISSING_REQUIRED_PARAMETER`, `PARAMETER_OUT_OF_RANGE`, `TOO_MANY_ARGUMENTS`, `UNDEFINED_ENUMERATION`, `UNRECOGNIZED_SERVICE`, `INVALID_DATA_ENCODING`

#### RejectReason.from_raw

```python
@classmethod
def from_raw(cls, value: int) -> RejectReason
```

The member for `value`; a pseudo-member when it has no name.

#### RejectReason.to_raw

```python
def to_raw(self) -> int
```

The raw integer value.

### AbortReason

`class AbortReason(enum.IntEnum)`

BACnet abort reason enumeration (Clause 18.10).

An `enum.IntEnum` (#1816): members are ints, compare equal to their
numbers and hash like them, so an int works wherever a member does. A
value without a name (a proprietary or unknown one) is a pseudo-member:
`AbortReason(200)` and `AbortReason.from_raw(200)` both give one, equal to
200, with `name` None and repr `AbortReason(200)`. A number outside
0 to 255 raises ValueError.

12 constants:

`OTHER`, `BUFFER_OVERFLOW`, `INVALID_APDU_IN_THIS_STATE`, `PREEMPTED_BY_HIGHER_PRIORITY_TASK`, `SEGMENTATION_NOT_SUPPORTED`, `SECURITY_ERROR`, `INSUFFICIENT_SECURITY`, `WINDOW_SIZE_OUT_OF_RANGE`, `APPLICATION_EXCEEDED_REPLY_TIME`, `OUT_OF_RESOURCES`, `TSM_TIMEOUT`, `APDU_TOO_LONG`

#### AbortReason.from_raw

```python
@classmethod
def from_raw(cls, value: int) -> AbortReason
```

The member for `value`; a pseudo-member when it has no name.

#### AbortReason.to_raw

```python
def to_raw(self) -> int
```

The raw integer value.

### EngineeringUnits

`class EngineeringUnits(enum.IntEnum)`

BACnet engineering units (Clause 21, BACnetEngineeringUnits).

An `enum.IntEnum` (#1816): members are ints, compare equal to their
numbers and hash like them, so an int works wherever a member does. A
value without a name (a proprietary or unknown one) is a pseudo-member:
`EngineeringUnits(1000)` and `EngineeringUnits.from_raw(1000)` both give one, equal to
1000, with `name` None and repr `EngineeringUnits(1000)`. A number outside
0 to 4294967295 raises ValueError.

269 constants:

`METERS_PER_SECOND_PER_SECOND`, `SQUARE_METERS`, `SQUARE_CENTIMETERS`, `SQUARE_FEET`, `SQUARE_INCHES`, `CURRENCY1`, `CURRENCY2`, `CURRENCY3`, `CURRENCY4`, `CURRENCY5`, `CURRENCY6`, `CURRENCY7`, `CURRENCY8`, `CURRENCY9`, `CURRENCY10`, `MILLIAMPERES`, `AMPERES`, `AMPERES_PER_METER`, `AMPERES_PER_SQUARE_METER`, `AMPERE_SQUARE_METERS`, `DECIBELS`, `DECIBELS_MILLIVOLT`, `DECIBELS_VOLT`, `FARADS`, `HENRYS`, `OHMS`, `OHM_METER_SQUARED_PER_METER`, `OHM_METERS`, `MILLIOHMS`, `KILOHMS`, `MEGOHMS`, `MICROSIEMENS`, `MILLISIEMENS`, `SIEMENS`, `SIEMENS_PER_METER`, `TESLAS`, `VOLTS`, `MILLIVOLTS`, `KILOVOLTS`, `MEGAVOLTS`, `VOLT_AMPERES`, `KILOVOLT_AMPERES`, `MEGAVOLT_AMPERES`, `VOLT_AMPERES_REACTIVE`, `KILOVOLT_AMPERES_REACTIVE`, `MEGAVOLT_AMPERES_REACTIVE`, `VOLTS_PER_DEGREE_KELVIN`, `VOLTS_PER_METER`, `DEGREES_PHASE`, `POWER_FACTOR`, `WEBERS`, `AMPERE_SECONDS`, `VOLT_AMPERE_HOURS`, `KILOVOLT_AMPERE_HOURS`, `MEGAVOLT_AMPERE_HOURS`, `VOLT_AMPERE_HOURS_REACTIVE`, `KILOVOLT_AMPERE_HOURS_REACTIVE`, `MEGAVOLT_AMPERE_HOURS_REACTIVE`, `VOLT_SQUARE_HOURS`, `AMPERE_SQUARE_HOURS`, `JOULES`, `KILOJOULES`, `KILOJOULES_PER_KILOGRAM`, `MEGAJOULES`, `WATT_HOURS`, `KILOWATT_HOURS`, `MEGAWATT_HOURS`, `WATT_HOURS_REACTIVE`, `KILOWATT_HOURS_REACTIVE`, `MEGAWATT_HOURS_REACTIVE`, `BTUS`, `KILO_BTUS`, `MEGA_BTUS`, `THERMS`, `TON_HOURS`, `JOULES_PER_KILOGRAM_DRY_AIR`, `KILOJOULES_PER_KILOGRAM_DRY_AIR`, `MEGAJOULES_PER_KILOGRAM_DRY_AIR`, `BTUS_PER_POUND_DRY_AIR`, `BTUS_PER_POUND`, `JOULES_PER_DEGREE_KELVIN`, `KILOJOULES_PER_DEGREE_KELVIN`, `MEGAJOULES_PER_DEGREE_KELVIN`, `JOULES_PER_KILOGRAM_DEGREE_KELVIN`, `NEWTON`, `CYCLES_PER_HOUR`, `CYCLES_PER_MINUTE`, `HERTZ`, `KILOHERTZ`, `MEGAHERTZ`, `PER_HOUR`, `GRAMS_OF_WATER_PER_KILOGRAM_DRY_AIR`, `PERCENT_RELATIVE_HUMIDITY`, `MICROMETERS`, `MILLIMETERS`, `CENTIMETERS`, `KILOMETERS`, `METERS`, `INCHES`, `FEET`, `CANDELAS`, `CANDELAS_PER_SQUARE_METER`, `WATTS_PER_SQUARE_FOOT`, `WATTS_PER_SQUARE_METER`, `LUMENS`, `LUXES`, `FOOT_CANDLES`, `MILLIGRAMS`, `GRAMS`, `KILOGRAMS`, `POUNDS_MASS`, `TONS`, `GRAMS_PER_SECOND`, `GRAMS_PER_MINUTE`, `KILOGRAMS_PER_SECOND`, `KILOGRAMS_PER_MINUTE`, `KILOGRAMS_PER_HOUR`, `POUNDS_MASS_PER_SECOND`, `POUNDS_MASS_PER_MINUTE`, `POUNDS_MASS_PER_HOUR`, `TONS_PER_HOUR`, `MILLIWATTS`, `WATTS`, `KILOWATTS`, `MEGAWATTS`, `BTUS_PER_HOUR`, `KILO_BTUS_PER_HOUR`, `JOULE_PER_HOURS`, `HORSEPOWER`, `TONS_REFRIGERATION`, `PASCALS`, `HECTOPASCALS`, `KILOPASCALS`, `MILLIBARS`, `BARS`, `POUNDS_FORCE_PER_SQUARE_INCH`, `MILLIMETERS_OF_WATER`, `CENTIMETERS_OF_WATER`, `INCHES_OF_WATER`, `MILLIMETERS_OF_MERCURY`, `CENTIMETERS_OF_MERCURY`, `INCHES_OF_MERCURY`, `DEGREES_CELSIUS`, `DEGREES_KELVIN`, `DEGREES_KELVIN_PER_HOUR`, `DEGREES_KELVIN_PER_MINUTE`, `DEGREES_FAHRENHEIT`, `DEGREE_DAYS_CELSIUS`, `DEGREE_DAYS_FAHRENHEIT`, `DELTA_DEGREES_FAHRENHEIT`, `DELTA_DEGREES_KELVIN`, `YEARS`, `MONTHS`, `WEEKS`, `DAYS`, `HOURS`, `MINUTES`, `SECONDS`, `HUNDREDTHS_SECONDS`, `MILLISECONDS`, `NEWTON_METERS`, `MILLIMETERS_PER_SECOND`, `MILLIMETERS_PER_MINUTE`, `METERS_PER_SECOND`, `METERS_PER_MINUTE`, `METERS_PER_HOUR`, `KILOMETERS_PER_HOUR`, `FEET_PER_SECOND`, `FEET_PER_MINUTE`, `MILES_PER_HOUR`, `CUBIC_FEET`, `CUBIC_METERS`, `IMPERIAL_GALLONS`, `MILLILITERS`, `LITERS`, `US_GALLONS`, `CUBIC_FEET_PER_SECOND`, `CUBIC_FEET_PER_MINUTE`, `MILLION_STANDARD_CUBIC_FEET_PER_MINUTE`, `CUBIC_FEET_PER_HOUR`, `CUBIC_FEET_PER_DAY`, `STANDARD_CUBIC_FEET_PER_DAY`, `MILLION_STANDARD_CUBIC_FEET_PER_DAY`, `THOUSAND_CUBIC_FEET_PER_DAY`, `THOUSAND_STANDARD_CUBIC_FEET_PER_DAY`, `POUNDS_MASS_PER_DAY`, `CUBIC_METERS_PER_SECOND`, `CUBIC_METERS_PER_MINUTE`, `CUBIC_METERS_PER_HOUR`, `CUBIC_METERS_PER_DAY`, `IMPERIAL_GALLONS_PER_MINUTE`, `MILLILITERS_PER_SECOND`, `LITERS_PER_SECOND`, `LITERS_PER_MINUTE`, `LITERS_PER_HOUR`, `US_GALLONS_PER_MINUTE`, `US_GALLONS_PER_HOUR`, `DEGREES_ANGULAR`, `DEGREES_CELSIUS_PER_HOUR`, `DEGREES_CELSIUS_PER_MINUTE`, `DEGREES_FAHRENHEIT_PER_HOUR`, `DEGREES_FAHRENHEIT_PER_MINUTE`, `JOULE_SECONDS`, `KILOGRAMS_PER_CUBIC_METER`, `KILOWATT_HOURS_PER_SQUARE_METER`, `KILOWATT_HOURS_PER_SQUARE_FOOT`, `WATT_HOURS_PER_CUBIC_METER`, `JOULES_PER_CUBIC_METER`, `MEGAJOULES_PER_SQUARE_METER`, `MEGAJOULES_PER_SQUARE_FOOT`, `MOLE_PERCENT`, `NO_UNITS`, `NEWTON_SECONDS`, `NEWTONS_PER_METER`, `PARTS_PER_MILLION`, `PARTS_PER_BILLION`, `PASCAL_SECONDS`, `PERCENT`, `PERCENT_OBSCURATION_PER_FOOT`, `PERCENT_OBSCURATION_PER_METER`, `PERCENT_PER_SECOND`, `PER_MINUTE`, `PER_SECOND`, `PSI_PER_DEGREE_FAHRENHEIT`, `RADIANS`, `RADIANS_PER_SECOND`, `REVOLUTIONS_PER_MINUTE`, `SQUARE_METERS_PER_NEWTON`, `WATTS_PER_METER_PER_DEGREE_KELVIN`, `WATTS_PER_SQUARE_METER_DEGREE_KELVIN`, `PER_MILLE`, `GRAMS_PER_GRAM`, `KILOGRAMS_PER_KILOGRAM`, `GRAMS_PER_KILOGRAM`, `MILLIGRAMS_PER_GRAM`, `MILLIGRAMS_PER_KILOGRAM`, `GRAMS_PER_MILLILITER`, `GRAMS_PER_LITER`, `MILLIGRAMS_PER_LITER`, `MICROGRAMS_PER_LITER`, `GRAMS_PER_CUBIC_METER`, `MILLIGRAMS_PER_CUBIC_METER`, `MICROGRAMS_PER_CUBIC_METER`, `NANOGRAMS_PER_CUBIC_METER`, `GRAMS_PER_CUBIC_CENTIMETER`, `BECQUERELS`, `KILOBECQUERELS`, `MEGABECQUERELS`, `GRAY`, `MILLIGRAY`, `MICROGRAY`, `SIEVERTS`, `MILLISIEVERTS`, `MICROSIEVERTS`, `MICROSIEVERTS_PER_HOUR`, `MILLIREMS`, `MILLIREMS_PER_HOUR`, `DECIBELS_A`, `NEPHELOMETRIC_TURBIDITY_UNIT`, `PH`, `GRAMS_PER_SQUARE_METER`, `MINUTES_PER_DEGREE_KELVIN`, `DEGREES_LOVIBOND`, `ALCOHOL_BY_VOLUME`, `INTERNATIONAL_BITTERING_UNITS`, `EUROPEAN_BITTERNESS_UNITS`, `DEGREES_PLATO`, `SPECIFIC_GRAVITY`, `EUROPEAN_BREWING_CONVENTION`

#### EngineeringUnits.from_raw

```python
@classmethod
def from_raw(cls, value: int) -> EngineeringUnits
```

The member for `value`; a pseudo-member when it has no name.

#### EngineeringUnits.to_raw

```python
def to_raw(self) -> int
```

The raw integer value.

### AuditOperation

BACnet audit operation.

Typed Audit request mappings accept standard values 0..15 and proprietary
values 32..63. `from_raw()` remains a lossless enum-wrapper constructor;
reserved or wider values are rejected when used in a request mapping.

16 constants:

`READ`, `WRITE`, `CREATE`, `DELETE`, `LIFE_SAFETY`, `ACKNOWLEDGE_ALARM`, `DEVICE_DISABLE_COMM`, `DEVICE_ENABLE_COMM`, `DEVICE_RESET`, `DEVICE_BACKUP`, `DEVICE_RESTORE`, `SUBSCRIPTION`, `NOTIFICATION`, `AUDITING_FAILURE`, `NETWORK_CHANGES`, `GENERAL`

Properties:

- `name: str | None`: The constant's name (`"READ"`), or None for a value without one.

#### AuditOperation.from_raw

```python
@staticmethod
def from_raw(value: int) -> AuditOperation
```

#### AuditOperation.to_raw

```python
def to_raw(self) -> int
```

### EnableDisable

BACnet DeviceCommunicationControl enable/disable options (Clause 16.1).

3 constants:

`ENABLE`, `DISABLE`, `DISABLE_INITIATION`

Properties:

- `name: str | None`: The constant's name (`"ENABLE"`), or None for a value without one.

#### EnableDisable.from_raw

```python
@staticmethod
def from_raw(value: int) -> EnableDisable
```

#### EnableDisable.to_raw

```python
def to_raw(self) -> int
```

### ReinitializedState

BACnet ReinitializeDevice state options (Clause 16.4).

8 constants:

`COLDSTART`, `WARMSTART`, `START_BACKUP`, `END_BACKUP`, `START_RESTORE`, `END_RESTORE`, `ABORT_RESTORE`, `ACTIVATE_CHANGES`

Properties:

- `name: str | None`: The constant's name (`"COLDSTART"`), or None for a value without one.

#### ReinitializedState.from_raw

```python
@staticmethod
def from_raw(value: int) -> ReinitializedState
```

#### ReinitializedState.to_raw

```python
def to_raw(self) -> int
```

### Segmentation

BACnet segmentation support options (Clause 20.1.2.4).

4 constants:

`BOTH`, `TRANSMIT`, `RECEIVE`, `NONE`

Properties:

- `name: str | None`: The constant's name (`"BOTH"`), or None for a value without one.

#### Segmentation.from_raw

```python
@staticmethod
def from_raw(value: int) -> Segmentation
```

#### Segmentation.to_raw

```python
def to_raw(self) -> int
```

### EventState

BACnet event state enumeration (Clause 12).

6 constants:

`NORMAL`, `FAULT`, `OFFNORMAL`, `HIGH_LIMIT`, `LOW_LIMIT`, `LIFE_SAFETY_ALARM`

Properties:

- `name: str | None`: The constant's name (`"NORMAL"`), or None for a value without one.

#### EventState.from_raw

```python
@staticmethod
def from_raw(value: int) -> EventState
```

#### EventState.to_raw

```python
def to_raw(self) -> int
```

### AcknowledgmentFilter

GetEnrollmentSummary acknowledgment filter (Clause 13.11.1.1).

3 constants:

`ALL`, `ACKED`, `NOT_ACKED`

Properties:

- `name: str | None`: The constant's name (`"ALL"`), or None for a value without one.

#### AcknowledgmentFilter.from_raw

```python
@staticmethod
def from_raw(value: int) -> AcknowledgmentFilter
```

#### AcknowledgmentFilter.to_raw

```python
def to_raw(self) -> int
```

### EnrollmentSummaryEventStateFilter

GetEnrollmentSummary event-state filter (Clause 13.11.1.1).

5 constants:

`OFFNORMAL`, `FAULT`, `NORMAL`, `ALL`, `ACTIVE`

Properties:

- `name: str | None`: The constant's name (`"OFFNORMAL"`), or None for a value without one.

#### EnrollmentSummaryEventStateFilter.from_raw

```python
@staticmethod
def from_raw(value: int) -> EnrollmentSummaryEventStateFilter
```

#### EnrollmentSummaryEventStateFilter.to_raw

```python
def to_raw(self) -> int
```

### EventType

BACnet event type enumeration (Clause 12.12.6).

20 constants:

`CHANGE_OF_BITSTRING`, `CHANGE_OF_STATE`, `CHANGE_OF_VALUE`, `COMMAND_FAILURE`, `FLOATING_LIMIT`, `OUT_OF_RANGE`, `CHANGE_OF_LIFE_SAFETY`, `EXTENDED`, `BUFFER_READY`, `UNSIGNED_RANGE`, `ACCESS_EVENT`, `DOUBLE_OUT_OF_RANGE`, `SIGNED_OUT_OF_RANGE`, `UNSIGNED_OUT_OF_RANGE`, `CHANGE_OF_CHARACTERSTRING`, `CHANGE_OF_STATUS_FLAGS`, `CHANGE_OF_RELIABILITY`, `NONE`, `CHANGE_OF_DISCRETE_VALUE`, `CHANGE_OF_TIMER`

Properties:

- `name: str | None`: The constant's name (`"CHANGE_OF_BITSTRING"`), or None for a value without one.

#### EventType.from_raw

```python
@staticmethod
def from_raw(value: int) -> EventType
```

#### EventType.to_raw

```python
def to_raw(self) -> int
```

### MessagePriority

BACnet text message priority (Clause 16.5).

2 constants:

`NORMAL`, `URGENT`

Properties:

- `name: str | None`: The constant's name (`"NORMAL"`), or None for a value without one.

#### MessagePriority.from_raw

```python
@staticmethod
def from_raw(value: int) -> MessagePriority
```

#### MessagePriority.to_raw

```python
def to_raw(self) -> int
```

### VTClass

BACnet virtual terminal class for VT-Open (Clause 17.2).

7 constants:

`DEFAULT_TERMINAL`, `ANSI_X3_64`, `DEC_VT52`, `DEC_VT100`, `DEC_VT220`, `HP_700_94`, `IBM_3130`

Properties:

- `name: str | None`: The constant's name (`"DEFAULT_TERMINAL"`), or None for a value without one.

#### VTClass.from_raw

```python
@staticmethod
def from_raw(value: int) -> VTClass
```

#### VTClass.to_raw

```python
def to_raw(self) -> int
```

### LifeSafetyOperation

BACnet life safety operation codes (Clause 12.15.13, Table 12-54).

10 constants:

`NONE`, `SILENCE`, `SILENCE_AUDIBLE`, `SILENCE_VISUAL`, `RESET`, `RESET_ALARM`, `RESET_FAULT`, `UNSILENCE`, `UNSILENCE_AUDIBLE`, `UNSILENCE_VISUAL`

Properties:

- `name: str | None`: The constant's name (`"NONE"`), or None for a value without one.

#### LifeSafetyOperation.from_raw

```python
@staticmethod
def from_raw(value: int) -> LifeSafetyOperation
```

#### LifeSafetyOperation.to_raw

```python
def to_raw(self) -> int
```

## Errors

The exceptions methods raise.

### BacnetError

`class BacnetError(Exception)`

Base exception for all BACnet errors.

### BacnetProtocolError

`class BacnetProtocolError(BacnetError)`

Raised on BACnet protocol-level errors (Error PDU).

Some services answer with a structured error body that adds fields to the
class and code. Each such attribute is None unless the device's error
carried it.

Attributes:
error\_class: The BACnet error class, an `ErrorClass` member
(an int equal to the wire value, #1816).
error\_code: The BACnet error code, an `ErrorCode` member.
first\_failed\_element\_number: For an AddListElement or
RemoveListElement ChangeList-Error, or a CreateObject-Error, the
position (from 1) of the list element or initial value that
failed, or 0 when the request failed for another reason. A
server's write\_property\_local of a list whose object refuses one
element sets it to that element's position in the list written.
first\_failed\_write\_attempt: For a WritePropertyMultiple-Error, the
object, property and index of the first write that failed.
first\_failed\_subscription: For a SubscribeCOVPropertyMultiple-Error
about one COV reference, its object, property and index. None for
a general failure of the request.
vendor\_id: For a ConfirmedPrivateTransfer-Error, the vendor of the
private service.
service\_number: For a ConfirmedPrivateTransfer-Error, the private
service number.
error\_parameters: For a ConfirmedPrivateTransfer-Error, the encoded
vendor-defined error parameters, when the device sent any.
vt\_session\_identifiers: For a VT-Close error that lists them, the
local identifiers of the VT sessions that could not be closed.

Attributes:

- `error_class: ErrorClass`
- `error_code: ErrorCode`
- `first_failed_element_number: Optional[int]`
- `first_failed_write_attempt: Optional[ObjectPropertyReference]`
- `first_failed_subscription: Optional[ObjectPropertyReference]`
- `vendor_id: Optional[int]`
- `service_number: Optional[int]`
- `error_parameters: Optional[bytes]`
- `vt_session_identifiers: Optional[list[int]]`

### BacnetTimeoutError

`class BacnetTimeoutError(BacnetError)`

Raised when a BACnet operation times out.

### BacnetRejectError

`class BacnetRejectError(BacnetError)`

Raised when a BACnet request is rejected.

Attributes:
reason: The reject reason, a `RejectReason` member (an int equal
to the wire value, #1816).

Attributes:

- `reason: RejectReason`

### BacnetAbortError

`class BacnetAbortError(BacnetError)`

Raised when a BACnet transaction is aborted.

Attributes:
reason: The abort reason, an `AbortReason` member (an int equal
to the wire value, #1816).

Attributes:

- `reason: AbortReason`

### BacnetLogNotAdvancingError

`class BacnetLogNotAdvancingError(BacnetError)`

Raised when a paged log read can't advance (#1530).

The device answered a page by sequence number with records from before
the one asked for, as bacnet-stack 1.6.1 does past its sequence wrap, or
with none where its record counts say it holds that record. Read the log
by position instead.

Attributes:
requested: The sequence number asked for.
returned: What the device numbered the first record it sent; None
when it sent none.

Attributes:

- `requested: int`
- `returned: int | None`

### BacnetReadRangeViolationError

`class BacnetReadRangeViolationError(BacnetError)`

Raised when a strict `read_range` refuses an answer that breaks a
ReadRange rule (#1531). Read again with `validation="lenient"` to keep
the answer and the rules it broke.

Attributes:
rule: The rule broken, in snake case, such as
`"zero_first_sequence_number"`.

Attributes:

- `rule: str`

### BacnetBvlcNakError

`class BacnetBvlcNakError(BacnetError)`

Raised when a BBMD answers a management request (`read_bdt` and
its siblings, #1841) with a BVLC-Result NAK. No reply raises
`BacnetTimeoutError` instead, so a refusal and silence never share a
class.

Attributes:
result\_code: The BVLC result code it sent, such as 0x0010 (Write-BDT
refused), 0x0030 (registration refused) or 0x0050 (delete refused).

Attributes:

- `result_code: int`

### BacnetBvlcUnexpectedResponseError

`class BacnetBvlcUnexpectedResponseError(BacnetError)`

Raised when a management request (`read_bdt` and its siblings,
\#1841) is answered by a BVLC message it doesn't take, such as a success
result to a table read.

Attributes:
expected: The BVLC function code the request takes as its answer.
got: The BVLC function code that came.

Attributes:

- `expected: int`
- `got: int`

### BacnetTransportError

`class BacnetTransportError(BacnetError, OSError)`

Raised when a transport socket or I/O operation fails.

Covers a failed bind or listen (B/IP, B/IPv6, `ScHub.start`), a refused
or failed dial, and other socket errors. It is also an `OSError`:
`errno` is the operating system's code when it reported one, else the
code for the failure's kind (for example `errno.EADDRINUSE`), else
`None`; `strerror` is the message. `except OSError` and
`except BacnetError` both catch it. Unlike a bare `OSError` it is
not narrowed to `ConnectionRefusedError` and the like, so test
`errno`.

Attributes:

- `errno: int | None`
- `strerror: str | None`
- `sc_untrusted_peer: ScUntrustedPeerInfo | None`: Untrusted certificate evidence from the actual failed outbound TLS attempt. None when unavailable at the verification observation point. This is not an authenticated peer or a history of background reconnect failures.

### BacnetRouterCloseError

`class BacnetRouterCloseError(BacnetError)`

A settled cleanup failure. The router retains it and prevents restart.

Attributes:

- `failures: tuple[RouterCleanupFailure, ...]`

### ObjectPropertyReference

`class ObjectPropertyReference(TypedDict)`

The object, property and array index a structured error names.

Keys:

- `object_identifier: ObjectIdentifier`
- `property_identifier: PropertyIdentifier`
- `property_array_index: int | None`

## Module functions

Functions at the top of the `rusty_bacnet` module.

### enum_name

```python
def enum_name(property: PropertyIdentifier, value: int) -> str | None
```

The name of enumerated `value` as read from `property`
(`enum_name(PropertyIdentifier.EVENT_STATE, 3) == "HIGH_LIMIT"`), or
None when the property's enumeration isn't known from the property alone
(Present\_Value, vendor properties) or the number has no name in it.

### decode_log_records

```python
def decode_log_records(result: ReadRangeResult) -> list[dict[str, Any]]
```

Decode the records of a `read_range` result of a log's Log\_Buffer
(#1534).

Each record is a dict with `timestamp` as a `(date, time)` pair and
`datum`, a dict whose `kind` names the choice and whose key of the
same name holds its value: `log_status` (a `BitString` naming
`LOG_DISABLED`, `BUFFER_PURGED`, `LOG_INTERRUPTED`), `boolean`,
`real`, `enumerated`, `unsigned`, `signed`, `bitstring` (a
`BitString` of no known type), `null` (no value), `failure` (`(ErrorClass, ErrorCode)` members), `time_change` (seconds), `any` (the value's encoding),
`values` (a Trend Log Multiple record's list of such dicts),
`notification` (an Event Log record's event notification, its
`event_values` as their encoding) or `audit_notification` (an Audit
Log record, as `audit_log_query` shows it). A Trend Log record adds
`status_flags` (a `BitString` naming `IN_ALARM`, `FAULT`,
`OVERRIDDEN`, `OUT_OF_SERVICE`), or None.

Raises ValueError for a result that isn't a log's Log\_Buffer, or whose
item data doesn't decode as `item_count` records, naming the first
record that fails by index and offset.

### list_serial_ports

```python
def list_serial_ports() -> list[str]
```

List the serial ports the operating system reports, as names to pass as
`serial_port=` for `transport="mstp"` (`/dev/ttyUSB0`,
`/dev/cu.usbserial-1410`, `COM3`).

macOS lists them through IOKit, Windows through SetupAPI and the registry,
and Linux from sysfs, so a port another program has open is listed too.
An empty list means the system reports none. Raises OSError (or the
subclass for the failure's kind) if the operating system can't be asked.

### refresh_logging

```python
def refresh_logging() -> None
```

Read the `logging` levels again, so that a level lowered since the
last read forwards the stack's events it now allows.

The stack's log events go to the logger `rusty_bacnet.<module path>`,
`rusty_bacnet.bacnet_transport.bip` for `bacnet_transport::bip`; only
the crate-level names (`rusty_bacnet.bacnet_transport` and so on) are
stable. TRACE goes out at level 5, which importing `rusty_bacnet` names
`TRACE`. The package logger has a `NullHandler`, so nothing prints
until the application configures logging.

The bridge decides which events to forward from a copy of the levels,
taken at import, whenever a client, server, hub or endpoint starts, and
by this function, so a disabled level never takes the GIL. Raised levels
filter at delivery as well.

Records arrive from one background thread, in order, shortly after the
event, and carry the event's fields as `record.bacnet_fields`. Past
4,096 waiting records, newer ones are dropped and a warning on
`rusty_bacnet` counts them. At exit, records already queued are
delivered first (waiting up to two seconds).

### inspect_sc_pem

```python
def inspect_sc_pem(
    identity_pem: bytes | bytearray,
    *,
    peer_trust_pem: bytes | bytearray | None = None,
    private_key_pem: bytes | bytearray | None = None,
    inspected_at_unix: int | None = None,
) -> ScPemInspection
```

Inspect bounded PEM bytes synchronously. Time is a signed 64-bit integer second.

ValueError: malformed/empty/oversized supplied PEM. TypeError: wrong input type,
including bool time. OverflowError: time outside the signed 64-bit range.
DER defects and key/date observations are returned in the report.

### inspect_sc_pem_files

```python
def inspect_sc_pem_files(
    identity_path: str | os.PathLike[str],
    *,
    peer_trust_path: str | os.PathLike[str] | None = None,
    private_key_path: str | os.PathLike[str] | None = None,
    inspected_at_unix: int | None = None,
) -> ScPemInspection
```

Read selected files once and inspect them without network I/O; OSError retains errno.

None time samples the clock once. A later connection may read changed files.
Same interpretation and input limits as inspect\_sc\_pem; no chmod or startup gate.
<!-- generated:end -->
