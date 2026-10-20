//! Linux thread scheduling: `sched_setscheduler` and its read-back, for the
//! MS/TP transport's dedicated thread (#501). nix wraps `sched_setaffinity`
//! but not the scheduling policy, so the policy calls are here.
//!
//! On Linux each of these acts on the calling thread when given pid 0,
//! never on the whole process: the kernel schedules threads, and a pid of 0
//! names the caller's own thread id.

/// A `sched_param` holding `priority` and zeros elsewhere (musl's has
/// fields beyond the priority).
fn param(priority: libc::c_int) -> libc::sched_param {
    // SAFETY: `sched_param` is a plain C struct of integers, for which all
    // zeros is a valid value.
    let mut param: libc::sched_param = unsafe { std::mem::zeroed() };
    param.sched_priority = priority;
    param
}

/// Put the calling thread under `SCHED_FIFO` at `priority`.
///
/// # Errors
///
/// The OS error: `EPERM` without `CAP_SYS_NICE` or an `RLIMIT_RTPRIO` of
/// at least `priority`, `EINVAL` for a priority outside 1..=99.
pub(crate) fn set_fifo(priority: libc::c_int) -> std::io::Result<()> {
    let param = param(priority);
    // SAFETY: pid 0 is the calling thread, and `param` is a valid,
    // initialized `sched_param` borrowed for the call, which only reads it.
    let ret = unsafe { libc::sched_setscheduler(0, libc::SCHED_FIFO, &param) };
    if ret == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// The calling thread's scheduling policy, and whether it is `SCHED_FIFO`,
/// with its static priority.
///
/// # Errors
///
/// The OS error of either read.
pub(crate) fn current() -> std::io::Result<(bool, libc::c_int)> {
    // SAFETY: pid 0 is the calling thread; the call takes no pointers.
    let policy = unsafe { libc::sched_getscheduler(0) };
    if policy == -1 {
        return Err(std::io::Error::last_os_error());
    }
    let mut param = param(0);
    // SAFETY: pid 0 is the calling thread, and `param` is a valid,
    // writable `sched_param` borrowed for the call, which writes only it.
    let ret = unsafe { libc::sched_getparam(0, &mut param) };
    if ret == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((policy == libc::SCHED_FIFO, param.sched_priority))
}
