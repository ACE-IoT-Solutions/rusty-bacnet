# bacnet-bip-core

Portable BACnet/IP behaviour from Annex J of ASHRAE 135, for
[rusty-bacnet](https://github.com/jscott3201/rusty-bacnet). It sits above
`bacnet-bip-codec`, which has the wire format, and below a socket.

- `#![no_std]`, no allocator, no dependencies besides `bacnet-bip-codec`.
- Tables live in arrays whose size you pick with a const generic.
- Time is an `Instant` you pass to each call; the crate reads no clock.

## What it has

The `bbmd` module holds a BBMD's broadcast distribution table and foreign
device table. Setting a BDT validates and cleans it and adds the BBMD's own
row. Foreign-device registration applies time-to-live limits, a
per-address quota and reserved capacity, and asks your own admission check
(an access list, a rate limit) in between. Registrations expire on your
clock, with the 30-second grace period.

## License

MIT
