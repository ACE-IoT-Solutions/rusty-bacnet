# bacnet-router-core

Portable BACnet router behaviour from Clause 6 of ASHRAE 135, for
[rusty-bacnet](https://github.com/jscott3201/rusty-bacnet). It sits beside
the portable MS/TP and B/IP cores, `bacnet-mstp-core` and `bacnet-bip-core`,
and is meant to join their ports in firmware, and to run under the hosted
`BACnetRouter` of `bacnet-network`.

- `#![no_std]`, no allocator, no dependencies besides `bacnet-time`.
- NPDUs are read in place from your frame and written into your buffer.
- Time is an `Instant` you pass to each call; the crate reads no clock. It
  is `bacnet-time`'s, in nanoseconds, re-exported here, the same type the
  MS/TP and B/IP cores take, so firmware feeds all three one clock.

Not published yet: the router engine, the hosted router's move onto it and
a firmware router image come first (#1679).

## What it has

- `npdu`: a borrowed view of an NPDU's control information and a writer.
  The view accepts and refuses exactly what `bacnet-encoding`'s decoder
  does, field for field, which a property test checks; an address longer
  than any data link uses is reported apart, with what a router needs to
  address its reject.
- `message`: the network-layer message types and reject reasons, and views
  and writers for the messages' data: network lists, rejects, routing-table
  entries and the rest.

## Example

```rust
use bacnet_router_core::npdu::{Address, Npdu};

// Node 0x0A on network 1 sends an APDU to node 0x05 on network 9.
let frame = [0x01, 0x20, 0x00, 0x09, 0x01, 0x05, 0xFF, 0x10, 0x08];
let npdu = Npdu::parse(&frame).unwrap();
assert_eq!(npdu.destination.unwrap().network, 9);

// Toward the next router the destination stays, the arrival network and
// the sender's MAC become the source, and the hop count drops by one.
let relayed = Npdu {
    source: Some(Address { network: 1, mac: &[0x0A] }),
    hop_count: npdu.hop_count - 1,
    ..npdu
};
let mut out = [0u8; 32];
let len = relayed.write(&mut out).unwrap();
assert_eq!(len, relayed.encoded_len());
assert_eq!(
    &out[..len],
    &[0x01, 0x28, 0x00, 0x09, 0x01, 0x05, 0x00, 0x01, 0x01, 0x0A, 0xFE, 0x10, 0x08],
);
```

## License

MIT
