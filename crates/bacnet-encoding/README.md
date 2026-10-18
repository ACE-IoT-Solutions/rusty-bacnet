# bacnet-encoding

BACnet tag, primitive, APDU and NPDU codecs for the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace.
Encoding a value produces bytes; it does not send a message or establish that a
peer accepted it.

## Quickstart from development source

The tag-sink API below is development-source functionality. The workspace's
`0.13.0` version does not establish that this API is in the published `0.13.0`
crate. Use a checkout named `rusty-bacnet` beside your application directory,
adjusting the relative path if needed:

```toml
[dependencies]
bacnet-encoding = { path = "../rusty-bacnet/crates/bacnet-encoding" }
```

Write a Real tag header into caller-owned storage, then verify that a header
which cannot fit leaves the existing bytes intact. This writes headers only;
the four Real value bytes announced by the first header are not appended.

```rust
use bacnet_encoding::sink::SinkFull;
use bacnet_encoding::tags::{encode_opening_tag_to_sink, encode_tag_to_sink, TagClass};

fn main() -> Result<(), SinkFull> {
    let mut storage = [0xA5; 2];
    let mut tail = &mut storage[..];
    assert_eq!(encode_tag_to_sink(&mut tail, 4, TagClass::Application, 4)?, 1);
    let refused = encode_opening_tag_to_sink(&mut tail, 15).unwrap_err();
    assert_eq!((refused.needed, refused.available), (2, 1));
    assert_eq!(tail.len(), 1);
    assert_eq!(storage, [0x44, 0xA5]);
    Ok(())
}
```

The tag-sink functions append one complete header or return `SinkFull` without
changing the sink. They do not validate every tag number or datatype-specific
length, and their atomic write guarantee does not span several encoder calls.
The primitive, APDU and NPDU modules retain their documented buffer APIs.

## Features and allocation

One of `std` or `alloc` is required, even when output uses a slice or a heapless
container. Disabling both is a compile error.

| Feature | Effect |
| --- | --- |
| `std` (default) | Hosted support; enables `alloc` and the dependencies' standard-library support. Adds tracing for reserved NPDU control bits. |
| `alloc` | `no_std` support with allocation. Firmware must supply a global allocator. |
| `heapless` | Adds `sink::EncodeSink` for `heapless::Vec<u8, N>`. This does not select `std` or `alloc` for you. |

`EncodeSink` also supports caller slices, `Vec<u8>` and `bytes::BytesMut`.
A bounded output adapter does not make this crate allocation-free.

For an alloc-only dependency build, replace the dependency block with:

```toml
[dependencies]
bacnet-encoding = { path = "../rusty-bacnet/crates/bacnet-encoding", default-features = false, features = ["alloc"] }
```

The quickstart executes unchanged with either block on a host. Firmware also
needs a global allocator and a compatible `bytes` configuration. On targets
without atomic compare-and-swap, such as ARMv6-M, enable `bytes`'s
`extra-platforms` feature and select an appropriate `portable-atomic` backend
in the firmware. The alloc-only block alone does not configure that backend.
Host doctests do not establish bare-metal execution or a no-heap profile.
Cargo features are additive, so check whether another dependency enables `std`.

## API documentation

This README is included in the crate's Rust documentation. The docs.rs profile
explicitly enables `std` and `heapless`, disables implicit defaults, and renders
only the default `x86_64-unknown-linux-gnu` target. That makes the optional sink
adapter visible through its actual feature gate; it does not claim the crate
builds without an allocator or qualify a bare-metal target.
