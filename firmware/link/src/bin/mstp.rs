//! The firmware link test's MS/TP image (#1626).
//!
//! On a bare-metal target this is a `no_main` image without a global
//! allocator. Its reset entry runs [`firmware_link::mstp::run`] once, with
//! the scenario's storage in a static; the core, which borrows its large
//! buffers from that storage, is built on the stack. Its input and its
//! summary pass through [`core::hint::black_box`], so none of the work can
//! be optimized away, and then it loops. Nothing runs it: building it proves
//! that the codec and the core link without `std` or an allocator, and
//! `scripts/ci/check-firmware-link.sh` checks its sizes. On a host the
//! binary is a stub, so workspace builds keep working; `tests/mstp.rs` runs
//! the scenario there.
//!
//! The only `unsafe` code is what the runtimes need: the code their entry
//! macros generate, and on RISC-V one block that borrows the static, which
//! cortex-m-rt's macro does safely on Cortex-M.

#![cfg_attr(target_os = "none", no_std, no_main)]

/// Cortex-M (ARMv6-M and ARMv7-M), on cortex-m-rt.
#[cfg(all(target_os = "none", target_arch = "arm"))]
mod image {
    use core::hint::black_box;

    use firmware_link::mstp::{run, Memory};

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

    use firmware_link::mstp::{run, Memory};

    static mut MEMORY: Memory = Memory::new();

    /// The reset entry. riscv-rt's macro has no counterpart to cortex-m-rt's
    /// `static mut` handling, so the static is borrowed by hand.
    #[riscv_rt::entry]
    fn main() -> ! {
        // SAFETY: this `&mut` is the only reference to the static for the
        // rest of the program. The image targets single-hart parts (which is
        // why riscv-rt's `single-hart` feature, which only drops its
        // multi-hart gate, is on), so one hart exists, its startup code
        // calls `main` once, and `main` never returns. No interrupt handler
        // can run either: the startup code writes 0 to `mie`, `mstatus.MIE`
        // is clear at reset, and nothing here enables either. Nothing else
        // names the static.
        #[allow(unsafe_code)]
        let memory = unsafe { &mut *addr_of_mut!(MEMORY) };
        black_box(run(black_box(memory)));
        loop {
            core::hint::spin_loop();
        }
    }
}

/// A host has nothing to flash: `tests/mstp.rs` runs the scenario.
#[cfg(not(target_os = "none"))]
fn main() {}
