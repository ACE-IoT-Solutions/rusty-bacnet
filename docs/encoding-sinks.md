# Encoding into caller-owned storage

The first sink-based encoders cover tag headers and context opening/closing
markers. They let a caller choose storage while keeping the existing
`BytesMut` entrypoints and their output unchanged. This is the first part of
#1614; the other encoder families still use their current buffers.

## Output contract

`bacnet_encoding::sink::EncodeSink` has two operations:

- `remaining_capacity(&self) -> Option<usize>` reports the bytes that can still
  be appended. `Some(n)` is a bounded sink's remaining room; `None` means it
  does not advertise a fixed bound. This is independent of a growable
  container's currently allocated capacity.
- `write(&mut self, bytes: &[u8]) -> Result<(), SinkFull>` appends the complete
  slice, or returns an error without changing output bytes or the output
  position. Implementers must preserve this per-write atomicity.

`SinkFull { needed: usize, available: usize }` describes the refused append,
without a string allocation. An encoder checks advertised capacity before
calling `write`. A sink can still refuse the write; its error is returned
unchanged. A custom sink with no advertised bound must also leave its output
unchanged on failure. This contract is for byte storage, not partial I/O.

The new `tags::encode_tag_to_sink`, `encode_opening_tag_to_sink` and
`encode_closing_tag_to_sink` return the number of appended bytes on success.
Each builds its complete header in at most seven stack bytes and makes one
write. A capacity refusal therefore leaves the entire header unwritten,
including when earlier headers already occupy the destination. Opening and
closing markers need at most two bytes.

The contract is atomic per write. It does not promise rollback across several
successful writes followed by a failure. A total-capacity preflight can prevent
a known shortage, but cannot roll back earlier writes if an arbitrary custom sink refuses a later one. A later
encoder family must stage its whole output or document its partial-output
behavior before adopting this interface. This change adds no rollback,
seek or allocation interface.

## Storage adapters

| Storage | Capacity report | Successful append |
| --- | --- | --- |
| `&mut [u8]` | Current slice length | Writes at the front and advances the slice to its unused tail |
| `heapless::Vec<u8, N>` | `N - len()` | Extends after existing elements; enabled by the `heapless` feature |
| `alloc::vec::Vec<u8>` | `None` | Extends after existing elements and may allocate |
| `bytes::BytesMut` | `None` | Extends after existing bytes and may allocate |

The bounded adapters preserve their bytes, length and remaining slice on a
failed append. The growable adapters retain their ordinary allocation-failure
behavior; `SinkFull` is not an allocation-error abstraction. The existing
`encode_tag`, `encode_opening_tag` and `encode_closing_tag` functions delegate
to the new implementation using the infallible `BytesMut` sink adapter.

## Scope and feature limits

This migration preserves the old encoders' treatment of all input values.
It adds no semantic validation of tag numbers, application datatype rules or
length limits. Valid-wire golden cases and legacy unvalidated-input cases are
tested separately. A decoder's configured length bound does not limit a
header encoder.

`bacnet-encoding` still requires either `std` or `alloc`, even when using a
slice or heapless output. The optional `heapless` adapter does not turn the
rest of the crate into an allocation-free codec. The sink contract and tag
header assembly themselves need no allocator. They do not depend on the
MS/TP codec; its existing `FrameSink` only informs the append convention.
