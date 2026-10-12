//! The scenarios that the firmware link test's images run (#1626, #1635).
//!
//! Each image links one stack into a bare-metal binary, the first two with
//! no allocator:
//!
//! - `mstp` ([`mstp::run`]): `bacnet-mstp-codec` and `bacnet-mstp-core`.
//! - `bip` ([`bip::run`]): `bacnet-bip-codec`, B/IP and B/IPv6, and
//!   `bacnet-bip-core`'s BBMD.
//! - `apdu` (`apdu::run`, with the `alloc` feature): `bacnet-encoding` and
//!   `bacnet-services`, `no_std` with `alloc`, and a global allocator over a
//!   fixed arena.
//!
//! One image per stack keeps each stack's flash and RAM separate: within one
//! LTO image, code can't be attributed to a crate reliably.
//!
//! A scenario is one `no_std` function that returns a summary of what it
//! saw, and each module holds the summary of a good run: [`mstp::EXPECTED`],
//! [`bip::EXPECTED`] and `apdu::expected()`. The host tests in `tests/` run
//! each scenario natively and compare the whole summary with it; that is
//! where behaviour is checked. Each image (`src/bin/`) calls its scenario
//! from its reset entry, so building it proves that the stack links without
//! `std`, and for `mstp` and `bip` without an allocator. CI never runs the
//! images. Their emulation build, with the `emulate` feature and a board's,
//! runs under QEMU by hand: each image checks its summary against the same
//! expected one and reports through semihosting (docs/ci.md, "Emulation
//! (manual)").
//!
//! The `alloc` feature, on by default, brings in the `apdu` module and its
//! dependencies. The `mstp` and `bip` images build without it: with it, this
//! crate links `alloc`, and an image without a global allocator fails to
//! link.
//!
//! The crate is `no_std` on every target, so the host build compiles the
//! same code as the images.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
pub mod apdu;
pub mod bip;
pub mod mstp;

/// Stop where the panic happened. The handler never reads the panic's
/// message, so the formatting code behind it can be left out of an image.
/// The host tests get std's handler instead, and the emulation build the
/// images' own, which reports the panic through semihosting.
#[cfg(all(target_os = "none", not(feature = "emulate")))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
