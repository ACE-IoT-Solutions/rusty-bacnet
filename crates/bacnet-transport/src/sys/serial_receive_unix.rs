//! Owned read-only serial descriptors. No transmit operation is exposed.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, IntoRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;

#[derive(Debug)]
pub(crate) struct Input {
    file: Option<File>,
    exclusive: bool,
}

impl Input {
    pub(crate) fn open(path: &str, baud: u32, rts: Option<bool>) -> io::Result<Self> {
        if baud == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "zero baud rate",
            ));
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC)
            .open(path)
            .map_err(|error| phase(error, "open read-only descriptor"))?;
        let mut input = Self {
            file: Some(file),
            exclusive: false,
        };
        let configured = input.configure(baud, rts);
        if let Err(error) = configured {
            return crate::mstp::monitor::combine(Err(error), input.close()).map(|()| input);
        }
        Ok(input)
    }

    fn configure(&mut self, baud: u32, rts: Option<bool>) -> io::Result<()> {
        let fd = self.as_raw_fd();
        // SAFETY: fd is live; flock takes no pointers.
        check(unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) })
            .map_err(|error| phase(error, "exclusive flock"))?;
        // SAFETY: TIOCEXCL takes no third argument and fd is live.
        check(unsafe { libc::ioctl(fd, libc::TIOCEXCL as _) })
            .map_err(|error| phase(error, "exclusive terminal ownership"))?;
        self.exclusive = true;
        super::serial_receive_baud::configure(fd, baud)
            .map_err(|error| phase(error, "configure baud and raw input"))?;
        if let Some(level) = rts {
            let bits = libc::TIOCM_RTS;
            let command = if level {
                libc::TIOCMBIS
            } else {
                libc::TIOCMBIC
            };
            // SAFETY: command reads one initialized integer; fd remains owned.
            check(unsafe { libc::ioctl(fd, command as _, &bits) })
                .map_err(|error| phase(error, "set RTS"))?;
        }
        // SAFETY: fd is owned; TCIFLUSH discards input only.
        check(unsafe { libc::tcflush(fd, libc::TCIFLUSH) })
            .map_err(|error| phase(error, "purge input"))
    }

    pub(crate) fn close(&mut self) -> io::Result<()> {
        let Some(file) = self.file.take() else {
            return Ok(());
        };
        let fd = file.into_raw_fd();
        let unlock = if self.exclusive {
            // SAFETY: fd was just transferred from File and is still live.
            check(unsafe { libc::ioctl(fd, libc::TIOCNXCL as _) })
                .map_err(|error| phase(error, "release terminal exclusivity"))
        } else {
            Ok(())
        };
        self.exclusive = false;
        // SAFETY: sole ownership was transferred above; close exactly once,
        // including on EINTR, to avoid closing a reused descriptor.
        let closed =
            check(unsafe { libc::close(fd) }).map_err(|error| phase(error, "close descriptor"));
        crate::mstp::monitor::combine(unlock, closed)
    }
}
impl AsRawFd for Input {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_ref().expect("live serial input").as_raw_fd()
    }
}
impl io::Read for Input {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.file
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "closed serial input"))?
            .read(bytes)
            .map_err(|error| phase(error, "read input"))
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
pub(super) fn check(result: libc::c_int) -> io::Result<()> {
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn phase(error: io::Error, phase: &str) -> io::Error {
    crate::mstp::monitor::context(error, phase.into())
}
