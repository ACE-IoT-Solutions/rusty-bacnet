# bacnet-client

Run a standalone asynchronous BACnet client with [`client::BACnetClient`] in
the [Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace. It owns
a transport and request lifecycle, with property/service APIs, discovery,
transaction management and segmentation. Use [`client::ClientConfig`] for
request settings or a transport-specific builder for connection setup.

This hosted crate requires `std`, Tokio and allocating state. It has no
alloc-only or no-heap profile. For one device combining client and server roles
under one transport owner, see the separate `bacnet-endpoint` crate and its
narrower service contract.

## Read a property using development source

Place an application directory beside a checkout named `rusty-bacnet`, then
add this exact dependency block to the application's `Cargo.toml`. These paths
refer to development source; they do not imply the shown APIs are published.
Adjust paths for a different layout. Tokio uses its current-thread runtime,
macros and time support; the program needs no multi-thread runtime feature.

```toml
[dependencies]
bacnet-client = { path = "../rusty-bacnet/crates/bacnet-client", default-features = false }
bacnet-server = { path = "../rusty-bacnet/crates/bacnet-server", default-features = false }
bacnet-objects = { path = "../rusty-bacnet/crates/bacnet-objects" }
bacnet-transport = { path = "../rusty-bacnet/crates/bacnet-transport" }
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types" }
tokio = { version = "1", features = ["macros", "rt", "time"] }
```

Save this complete program as `src/main.rs`. It initializes an Analog Input to
42.0 degrees Celsius, seeds a Device and its Object_List with
`build_database_with_extra`, and starts a real client and server over bounded
in-process loopback channels. The read has a two-second outer timeout, a
one-second APDU timeout and no retries. Its acknowledgement is checked against
independent application-tagged REAL bytes for 42.0.

```rust
use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device_identity::{build_database_with_extra, DeviceIdentity};
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use tokio::time::{timeout, Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let identity = DeviceIdentity::new(1001, 42)?;
    let mut analog = AnalogInputObject::new(
        7,
        "supply-temperature",
        EngineeringUnits::DEGREES_CELSIUS.to_raw(),
    )?;
    analog.set_present_value(42.0);
    let database = build_database_with_extra(&identity, vec![Box::new(analog)])?;
    assert!(database.get(&identity.device_oid()).is_some());

    let (client_port, server_port) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut server = BACnetServer::generic_builder()
        .device(identity)
        .database(database)
        .transport(server_port)
        .build()
        .await?;
    let config = ClientConfig {
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let mut client = BACnetClient::start(config, client_port).await?;
    let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7)?;
    let reply = timeout(
        Duration::from_secs(2),
        client.read_property(&[2], object, PropertyIdentifier::PRESENT_VALUE, None),
    )
    .await;

    let client_stopped = client.stop().await;
    let server_stopped = server.stop().await;
    let ack = reply??;
    client_stopped?;
    server_stopped?;
    assert_eq!(ack.object_identifier, object);
    assert_eq!(ack.property_identifier, PropertyIdentifier::PRESENT_VALUE);
    assert_eq!(ack.property_array_index, None);
    assert_eq!(ack.property_value, [0x44, 0x42, 0x28, 0x00, 0x00]);
    Ok(())
}
```

Both owners are explicitly stopped before the read result is propagated. The
read timeout does not impose a shutdown deadline. Directly setting the analog
value is initialization before server ownership; live updates need the
server's local mutation APIs and their event/COV processing.

Loopback sends to its paired peer regardless of the requested destination.
This program establishes an in-process request/response exchange and cleanup,
not destination filtering, UDP, TLS, serial, physical equipment, timing,
firmware behavior or BACnet conformance. Enabling an optional feature while
running it does not exercise that feature's network transport.

## Features and builders

| Feature selection | Public entry points |
|---|---|
| Default or `--no-default-features` | `BACnetClient::start`, `BACnetClient::generic_builder` and `BACnetClient::bip_builder`; hosted `std`/Tokio/allocation remain required. |
| `ipv6` | Adds `Bip6ClientBuilder` and `BACnetClient::bip6_builder` for BACnet/IPv6. |
| `sc-tls` | Adds `ScClientBuilder` and `BACnetClient::sc_builder` for a constrained-TLS BACnet/SC hub connection. |

For the displayed application, select `--features bacnet-client/ipv6`,
`--features bacnet-client/sc-tls`, or
`--features bacnet-client/ipv6,bacnet-client/sc-tls` when running Cargo. Each
selection still runs the loopback program above. Real connections need the
appropriate transport addresses and, for SC, validated TLS policy and a
caller-provisioned nonzero device UUID. TLS peer authentication does not by
itself authorize BACnet services or mutations on a server.

The docs.rs profile enables `ipv6` and `sc-tls`, disables defaults and targets
`x86_64-unknown-linux-gnu`. Optional builders and factories carry feature badges;
the always-present `client` module and generic builder stay available under
each profile. This README is the crate landing page and its program runs as a
doctest. A local render does not establish a docs.rs deployment.

## Request and lifecycle ownership

[`client::BACnetClient::read_property`] returns encoded property data and checks
that the acknowledgement matches the requested object, property and array
index. Decode it for your property's expected type; the literal above is an
independent expectation for this one REAL value, not a general decoder.

Keep the client owner until requests finish, then call
[`client::BACnetClient::stop`]. Stop aborts and joins its dispatch and local
network-number work, cancels transactions and stops the owned transport. A
cancelled stop waiter retains joins for a later stop; dropping the client is
not a substitute for awaiting cleanup. The API does not promise a universal
fixed shutdown deadline.

The paired server's database exposes raw object state; use its `read_local`
method for executor-owned live properties. Release database guards before
network requests or shutdown. Server service/mutation/source policies remain
application decisions, including when a transport authenticates its peers.
