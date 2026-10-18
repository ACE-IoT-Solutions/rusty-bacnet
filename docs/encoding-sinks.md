# Encoding into caller-owned storage

Tag headers, context opening/closing markers and the bounded scalar primitive
family support caller-selected storage. Their existing `BytesMut` entrypoints
and output remain unchanged. These are parts of #1614; variable-length and
composite encoder families still use their current buffers.

## Output contract

`bacnet_encoding::sink::EncodeSink` has two operations:

- `remaining_capacity(&self) -> Option<usize>` reports the bytes that can still
  be appended. `Some(n)` is a bounded sink's remaining room; `None` means it
  does not advertise a fixed bound. This is independent of a growable
  container's currently allocated capacity.
- `write(&mut self, bytes: &[u8]) -> Result<(), SinkFull>` appends the complete
  slice, or returns an error without changing output bytes or the output
  position. Implementers must preserve this per-write atomicity.

`SinkFull { needed: usize, available: usize }` describes the refused append
without a string allocation. Each sink encoder returns `Result<usize,
SinkFull>`, reporting the appended byte count on success. A known shortage
returns the total required and available lengths without calling `write`.
A custom sink can still refuse a write despite unknown or sufficient advertised
capacity; its error is returned unchanged. This contract is for byte storage,
not partial I/O.

The functions `tags::encode_tag_to_sink`, `encode_opening_tag_to_sink` and
`encode_closing_tag_to_sink` stage a complete header and make one write. Tag
headers occupy at most seven bytes, and opening/closing markers at most two.

Each bounded scalar `_to_sink` function also makes one caller write for its
**whole value**, including any tag and length octets. Tagged values use an
eleven-byte staging buffer: up to eight content bytes and three header bytes.
A refusal leaves earlier output, the slice cursor, container length and unused
tail unchanged. Header and payload are never issued as separate caller writes.
The eleven-byte output bound does not describe the full call-stack requirement.

Atomicity stops at one encoder call. Several successful calls followed by a
refusal retain the completed earlier values. A total-capacity preflight does
not provide rollback if an arbitrary custom sink refuses a later write. No
rollback, seek or allocation interface is added.

## Scalar entrypoints and wire behavior

The following existing `primitives` functions each have a matching `_to_sink`
entrypoint. Their sink argument accepts `EncodeSink + ?Sized`.

| Form | Existing names with the new suffix available |
| --- | --- |
| Raw contents | `encode_unsigned`, `encode_signed`, `encode_real`, `encode_double` |
| Application tagged | `encode_app_null`, `encode_app_boolean`, `encode_app_unsigned`, `encode_app_signed`, `encode_app_real`, `encode_app_double`, `encode_app_enumerated`, `encode_app_date`, `encode_app_time`, `encode_app_object_id` |
| Context tagged | `encode_ctx_unsigned`, `encode_ctx_signed`, `encode_ctx_real`, `encode_ctx_double`, `encode_ctx_enumerated`, `encode_ctx_boolean`, `encode_ctx_object_id`, `encode_ctx_date` |

Unsigned and Signed retain their minimum big-endian widths and the existing
`unsigned_len` / `signed_len` results. Enumerated uses the unsigned layout.
Real and Double preserve their IEEE-754 payload bits in big-endian order.
Application Null has no contents; application Boolean carries its value in the
header, whereas context Boolean has one content byte. Date and Time retain
their four fields, including wildcard bytes; ObjectIdentifier retains its
10-bit type and 22-bit instance. Tag lengths and these scalar layouts follow
ASHRAE 135-2020 Clauses 20.2.1.3, 20.2.2–20.2.7 and 20.2.11–20.2.15.

For example, context Unsigned 256 under tag 15 occupies four bytes:

```rust
use bacnet_encoding::primitives::encode_ctx_unsigned_to_sink;

let mut storage = [0xA5; 4];
let mut tail = &mut storage[..];
assert_eq!(encode_ctx_unsigned_to_sink(&mut tail, 15, 256).unwrap(), 4);
assert!(tail.is_empty());
assert_eq!(storage, [0xFA, 0x0F, 0x01, 0x00]);
```

The corresponding public rustdoc example is executable in the default and
alloc-only profiles. Independent literal wire cases cover widths, boundaries,
inline and extended context tags; a separate unchanged baseline probe checks
legacy `BytesMut` output. Compatibility cases separately preserve tag 255,
unvalidated Date/Time fields and nonfinite floating-point payloads. Those cases
do not advertise the inputs as valid in every BACnet service context.

## Storage adapters

| Storage | Capacity report | Successful append |
| --- | --- | --- |
| `&mut [u8]` | Current slice length | Writes at the front and advances the slice to its unused tail |
| `heapless::Vec<u8, N>` | `N - len()` | Extends after existing elements; enabled by the `heapless` feature |
| `alloc::vec::Vec<u8>` | `None` | Extends after existing elements and may allocate |
| `bytes::BytesMut` | `None` | Extends after existing bytes and may allocate |

The bounded adapters preserve their bytes, length and remaining slice on a
failed append. Growable adapters retain their ordinary allocation-failure
behavior; `SinkFull` is not an allocation-error abstraction. Existing tag and
scalar `BytesMut` functions delegate through that infallible sink adapter.

## Scope and feature limits

This migration adds no semantic validation or new raw/context variants.
Strings, octet strings, bit strings, PropertyValue/composite dispatch,
timestamps and decoders retain their existing APIs and behavior. Higher-level
callers use the same scalar `BytesMut` paths; no whole-message atomicity is
implied by composing them. A decoder's configured length bound does not limit
a header encoder.

`bacnet-encoding` still requires either `std` or `alloc`, including with slice
or heapless output. The optional adapter does not make the rest of the crate
allocation-free. Sink storage selection and bounded staging do not establish
physical-device operation or a performance improvement. The sink contract,
tag assembly and bounded scalar assembly themselves need no allocator.
