# bacnet-types

BACnet identifiers, enum values, addresses, property values and errors for the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace.
These are protocol data types; this crate does not open a transport or run a device.

## Quickstart from development source

The dependency blocks below use a checkout named `rusty-bacnet` beside your
application directory. They describe the development source, not the contents
of a published crate version. Adjust the relative path for another layout.

```toml
[dependencies]
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types" }
```

Construct an identifier for a concrete object, check its wire bytes, and decode
it again. `new_addressable` rejects the wildcard instance as well as values
outside the field widths; `new` and `decode` also support the valid wire wildcard.

```rust
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

fn main() -> Result<(), bacnet_types::error::Error> {
    let object = ObjectIdentifier::new_addressable(ObjectType::ANALOG_INPUT, 7)?;
    let wire = object.encode();
    assert_eq!(wire, [0x00, 0x00, 0x00, 0x07]);
    assert_eq!(ObjectIdentifier::decode(&wire)?, object);
    assert_eq!(object.instance_number(), 7);
    assert!(ObjectIdentifier::new(
        ObjectType::DEVICE,
        ObjectIdentifier::MAX_INSTANCE + 1,
    ).is_err());
    Ok(())
}
```

## Features and allocation

One of `std` or `alloc` is required. Disabling both is a compile error; an
allocation-free configuration is not available.

| Feature | Effect |
| --- | --- |
| `std` (default) | Hosted support, including `error::Error::Transport` and `bip_port`. Enables `alloc`. |
| `alloc` | `no_std` support with allocation. Firmware must supply a global allocator. |
| `serde` | Deserialization for `enums::ObjectType` and `enums::PropertyIdentifier`; enables `alloc`, without selecting `std`. |

Enum newtypes support `FromStr` for their named constants in common case styles,
such as `analog-input` and `ANALOG_INPUT`. The normal runtime dependencies are
`bitflags`, `smallvec` and `thiserror`, plus `serde` when selected.

For an alloc-only dependency build, replace the dependency block with:

```toml
[dependencies]
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types", default-features = false, features = ["alloc"] }
```

The same quickstart runs on a host with either block. Running it on a host with
alloc-only dependencies is not a bare-metal execution test or proof that the
crate needs no heap. A firmware application also supplies its runtime and
allocator. Cargo features are additive: another dependency can enable `std`
again, so inspect the application's resolved feature graph.

## API documentation

This README is also the crate's Rust documentation landing page. The docs.rs
profile explicitly enables `std` and `serde`, disables implicit defaults, and
renders only the default `x86_64-unknown-linux-gnu` target. Documented items
follow their real feature gates; this profile does not establish firmware
qualification. The library's module pages describe the individual types.
