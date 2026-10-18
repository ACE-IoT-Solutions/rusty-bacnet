# bacnet-services

BACnet service request and response parameter codecs for the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace.
Each module provides structures and byte encoders/decoders for a service family.
Property values at constructed boundaries remain raw bytes for the application
to interpret. This crate does not perform network requests or operate a device.

## Quickstart from development source

Use a checkout named `rusty-bacnet` beside your application directory. These
blocks describe development-source dependencies, not the contents of a published
release. The example imports `bacnet-types` and `bytes` directly, so both are
listed as application dependencies too. Adjust the paths for another layout.

```toml
[dependencies]
bacnet-services = { path = "../rusty-bacnet/crates/bacnet-services" }
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types" }
bytes = "1"
```

Encode and decode ReadProperty service parameters. These are only the service
body: no confirmed-request APDU, NPDU, transport frame or device interaction is
created by this example.

```rust
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

fn main() -> Result<(), bacnet_types::error::Error> {
    let request = ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new_addressable(ObjectType::ANALOG_INPUT, 7)?,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };
    let mut wire = BytesMut::new();
    request.encode(&mut wire);
    assert_eq!(&wire[..], &[0x0C, 0, 0, 0, 7, 0x19, 85]);
    assert_eq!(ReadPropertyRequest::decode(&wire)?, request);
    assert!(ReadPropertyRequest::decode(&wire[..1]).is_err());
    Ok(())
}
```

## Features and allocation

One of `std` or `alloc` is required. Disabling both is a compile error.
Owned decoded lists and values require allocation.

| Feature | Effect |
| --- | --- |
| `std` (default) | Hosted support; enables `alloc` and standard-library support in `bacnet-types`, `bacnet-encoding` and `bytes`. |
| `alloc` | `no_std` service codecs with allocation; enables the foundation crates' `alloc` profiles. Firmware must supply a global allocator. |

For alloc-only dependencies, replace the entire dependency block, disabling
defaults on every directly listed foundation crate and on `bytes`:

```toml
[dependencies]
bacnet-services = { path = "../rusty-bacnet/crates/bacnet-services", default-features = false, features = ["alloc"] }
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types", default-features = false, features = ["alloc"] }
bytes = { version = "1", default-features = false }
```

The same example executes on a host with either block. Firmware additionally
supplies an allocator and a compatible `bytes` atomic backend. A target without
atomic compare-and-swap, such as ARMv6-M, needs `bytes`'s `extra-platforms`
feature and a firmware-selected `portable-atomic` backend. The block above does
not choose that backend. Host alloc doctests are separate from bare-metal
execution and do not establish an allocation-free configuration. Cargo features
are additive; another dependency can still enable `std`.

## API documentation

This README is also the crate's Rust documentation landing page. The docs.rs
profile explicitly enables `std`, disables implicit defaults, and renders only
the default `x86_64-unknown-linux-gnu` target. It exposes the actual hosted API
without bypassing feature gates or claiming firmware qualification.
