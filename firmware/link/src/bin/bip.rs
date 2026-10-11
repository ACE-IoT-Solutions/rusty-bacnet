//! The firmware link test's B/IP image (#1626).
//!
//! On a bare-metal target this is a `no_main` image without a global
//! allocator. Its reset entry runs [`firmware_link::bip::run`] once, with the
//! scenario's storage in a static. Its input and its summary pass through
//! [`core::hint::black_box`], so none of the work can be optimized away,
//! and then it loops. Nothing runs it: building it proves that the BVLL
//! codec links without `std` or an allocator, and
//! `scripts/ci/check-firmware-link.sh` checks its sizes. On a host the
//! binary is a stub; `tests/bip.rs` runs the scenario there.
//!
//! The only `unsafe` code is what the runtimes need: the code their entry
//! macros generate, and on RISC-V one block that borrows the static, which
//! cortex-m-rt's macro does safely on Cortex-M.

#![cfg_attr(target_os = "none", no_std, no_main)]

/// Cortex-M (ARMv6-M and ARMv7-M), on cortex-m-rt.
#[cfg(all(target_os = "none", target_arch = "arm"))]
mod image {
    use core::hint::black_box;

    use firmware_link::bip::{run, Memory};

    /// The reset entry. cortex-m-rt's macro turns the `static mut` below
    /// into a `&'static mut`, which is sound because the entry runs once.
    #[cortex_m_rt::entry]
    fn main() -> ! {
        static mut MEMORY: Memory = Memory::new();
        black_box(run(black_box(MEMORY)));
        loop {
            core::hint::spin_loop();
        }
    }
}

/// RISC-V (RV32IMC, the ESP32-C3 class), on riscv-rt.
#[cfg(all(target_os = "none", target_arch = "riscv32"))]
mod image {
    use core::hint::black_box;
    use core::ptr::addr_of_mut;

    use firmware_link::bip::{run, Memory};

    static mut MEMORY: Memory = Memory::new();

    /// The reset entry. riscv-rt's macro has no counterpart to cortex-m-rt's
    /// `static mut` handling, so the static is borrowed by hand.
    #[riscv_rt::entry]
    fn main() -> ! {
        // SAFETY: this `&mut` is the only reference to the static for the
        // rest of the program. riscv-rt's startup code calls `main` once and
        // `main` never returns; with the `single-hart` feature no other hart
        // runs it; and no interrupt handler can run, because the startup
        // code writes 0 to `mie`, `mstatus.MIE` is clear at reset, and
        // nothing here enables either. Nothing else names the static.
        #[allow(unsafe_code)]
        let memory = unsafe { &mut *addr_of_mut!(MEMORY) };
        black_box(run(black_box(memory)));
        loop {
            core::hint::spin_loop();
        }
    }
}

/// A host has nothing to flash: `tests/bip.rs` runs the scenario.
#[cfg(not(target_os = "none"))]
fn main() {}
