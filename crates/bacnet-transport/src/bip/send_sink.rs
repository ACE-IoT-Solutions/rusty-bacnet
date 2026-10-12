//! A test stand-in for the network: it keeps every datagram a
//! [`BipSocket`](super::BipSocket) sends, with the Tokio time it went, or
//! fails each send on request.

use std::io;
use std::net::SocketAddrV4;
use std::sync::{Arc, Mutex, MutexGuard};

/// One datagram sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Sent {
    /// When it went, on the Tokio clock.
    pub(super) at: tokio::time::Instant,
    /// Where it went.
    pub(super) to: SocketAddrV4,
    /// What it held.
    pub(super) bytes: Vec<u8>,
}

#[derive(Debug, Default)]
struct State {
    sent: Vec<Sent>,
    fail: bool,
}

/// What a recording socket sent. Clones share one record.
#[derive(Clone, Debug, Default)]
pub(super) struct SendSink {
    state: Arc<Mutex<State>>,
}

impl SendSink {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Record one send, or fail it while [`fail_sends`](Self::fail_sends)
    /// is on.
    pub(super) fn send(&self, buf: &[u8], to: SocketAddrV4) -> io::Result<usize> {
        let mut state = self.lock();
        if state.fail {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "send refused by the test sink",
            ));
        }
        state.sent.push(Sent {
            at: tokio::time::Instant::now(),
            to,
            bytes: buf.to_vec(),
        });
        Ok(buf.len())
    }

    /// Fail every send from now on, or stop failing them.
    pub(super) fn fail_sends(&self, fail: bool) {
        self.lock().fail = fail;
    }

    /// Everything sent so far, which leaves the record.
    pub(super) fn take(&self) -> Vec<Sent> {
        std::mem::take(&mut self.lock().sent)
    }
}
