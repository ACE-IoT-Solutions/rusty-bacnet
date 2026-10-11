//! The MS/TP firmware link test's image (#1626).
//!
//! On a bare-metal target this is a `no_main` image without a global
//! allocator. Its reset entry runs [`mstp_link::run`] once, with the
//! scenario's storage and the core in statics, hands the summary to
//! [`core::hint::black_box`] so the work can't be optimized away, and loops.
//! Nothing runs it: building it proves that the codec and the core link
//! without `std` or an allocator, and `scripts/ci/check-firmware-link.sh`
//! checks its section sizes. On a host the binary is a stub, so workspace
//! builds keep working; `tests/scenario.rs` runs the scenario there.
//!
//! The only `unsafe` code is what the runtimes need: the code their entry
//! macros generate, and on RISC-V one block that borrows the statics, which
//! cortex-m-rt's macro does safely on Cortex-M.

#![cfg_attr(target_os = "none", no_std, no_main)]

/// Cortex-M (ARMv6-M and ARMv7-M), on cortex-m-rt.
#[cfg(all(target_os = "none", target_arch = "arm"))]
mod image {
    use core::mem::MaybeUninit;

    use mstp_link::{Core, Memory};

    /// The reset entry. cortex-m-rt's macro turns each `static mut` below
    /// into a `&'static mut`, which is sound because the entry runs once.
    #[cortex_m_rt::entry]
    fn main() -> ! {
        static mut MEMORY: Memory = Memory::new();
        static mut CORE: MaybeUninit<Core<'static>> = MaybeUninit::uninit();
        core::hint::black_box(mstp_link::run(MEMORY, CORE));
        loop {
            core::hint::spin_loop();
        }
    }
}

/// RISC-V (RV32IMC, the ESP32-C3 class), on riscv-rt.
#[cfg(all(target_os = "none", target_arch = "riscv32"))]
mod image {
    use core::mem::MaybeUninit;
    use core::ptr::addr_of_mut;

    use mstp_link::{Core, Memory};

    static mut MEMORY: Memory = Memory::new();
    static mut CORE: MaybeUninit<Core<'static>> = MaybeUninit::uninit();

    /// The reset entry. riscv-rt's macro has no counterpart to cortex-m-rt's
    /// `static mut` handling, so the statics are borrowed by hand.
    #[riscv_rt::entry]
    fn main() -> ! {
        // SAFETY: the entry runs once and never returns, and nothing else
        // names these statics, so each borrow is the only reference to its
        // static for the rest of the program.
        #[allow(unsafe_code)]
        let (memory, core) = unsafe { (&mut *addr_of_mut!(MEMORY), &mut *addr_of_mut!(CORE)) };
        core::hint::black_box(mstp_link::run(memory, core));
        loop {
            core::hint::spin_loop();
        }
    }
}

/// Stop where the panic happened. The handler never reads the panic's
/// message, so the formatting code behind it can be left out of the image.
#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

/// A host has nothing to flash: `tests/scenario.rs` runs the scenario.
#[cfg(not(target_os = "none"))]
fn main() {}
