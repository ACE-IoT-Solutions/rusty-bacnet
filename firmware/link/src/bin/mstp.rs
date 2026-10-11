//! The firmware link test's MS/TP image (#1626).
//!
//! On a bare-metal target this is a `no_main` image without a global
//! allocator. Its reset entry runs [`firmware_link::mstp::run`] once, with
//! the scenario's storage and the core in statics. Its inputs and its
//! summary pass through [`core::hint::black_box`], so none of the work can
//! be optimized away, and then it loops. Nothing runs it: building it proves
//! that the codec and the core link without `std` or an allocator, and
//! `scripts/ci/check-firmware-link.sh` checks its sizes. On a host the
//! binary is a stub, so workspace builds keep working; `tests/mstp.rs` runs
//! the scenario there.
//!
//! The only `unsafe` code is what the runtimes need: the code their entry
//! macros generate, and on RISC-V one block that borrows the statics, which
//! cortex-m-rt's macro does safely on Cortex-M.

#![cfg_attr(target_os = "none", no_std, no_main)]

/// Cortex-M (ARMv6-M and ARMv7-M), on cortex-m-rt.
#[cfg(all(target_os = "none", target_arch = "arm"))]
mod image {
    use core::hint::black_box;
    use core::mem::MaybeUninit;

    use firmware_link::mstp::{run, Core, Memory};

    /// The reset entry. cortex-m-rt's macro turns each `static mut` below
    /// into a `&'static mut`, which is sound because the entry runs once.
    #[cortex_m_rt::entry]
    fn main() -> ! {
        static mut MEMORY: Memory = Memory::new();
        static mut CORE: MaybeUninit<Core<'static>> = MaybeUninit::uninit();
        black_box(run(black_box(MEMORY), black_box(CORE)));
        loop {
            core::hint::spin_loop();
        }
    }
}

/// RISC-V (RV32IMC, the ESP32-C3 class), on riscv-rt.
#[cfg(all(target_os = "none", target_arch = "riscv32"))]
mod image {
    use core::hint::black_box;
    use core::mem::MaybeUninit;
    use core::ptr::addr_of_mut;

    use firmware_link::mstp::{run, Core, Memory};

    static mut MEMORY: Memory = Memory::new();
    static mut CORE: MaybeUninit<Core<'static>> = MaybeUninit::uninit();

    /// The reset entry. riscv-rt's macro has no counterpart to cortex-m-rt's
    /// `static mut` handling, so the statics are borrowed by hand.
    #[riscv_rt::entry]
    fn main() -> ! {
        // SAFETY: these two `&mut` are the only references to the statics
        // for the rest of the program. riscv-rt's startup code calls `main`
        // once and `main` never returns; with the `single-hart` feature no
        // other hart runs it; and no interrupt handler can run, because the
        // startup code writes 0 to `mie`, `mstatus.MIE` is clear at reset,
        // and nothing here enables either. Nothing else names the statics.
        #[allow(unsafe_code)]
        let (memory, core) = unsafe { (&mut *addr_of_mut!(MEMORY), &mut *addr_of_mut!(CORE)) };
        black_box(run(black_box(memory), black_box(core)));
        loop {
            core::hint::spin_loop();
        }
    }
}

/// A host has nothing to flash: `tests/mstp.rs` runs the scenario.
#[cfg(not(target_os = "none"))]
fn main() {}
