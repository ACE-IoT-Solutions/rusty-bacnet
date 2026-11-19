//! Read-only session status and observation of its own link.

use super::{EndpointSession, Lifecycle, SessionRole};
use bacnet_transport::port::TransportPort;
use bacnet_transport::tap::FrameTapReceiver;
use std::sync::atomic::Ordering;

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// Returns the composed session role (narrow admin).
    pub fn session_role(&self) -> SessionRole {
        self.role
    }

    /// Returns true while the session dispatch is running (narrow admin).
    pub fn is_running(&self) -> bool {
        self.lifecycle.load(Ordering::Acquire) == Lifecycle::Running as u8
    }

    /// Subscribe to raw frames on a B/IP or SC builder session's transport.
    ///
    /// Returns `None` outside the running state or when the builder did not
    /// supply a tap. Each receiver has its own bounded queue; full queues
    /// discard new frames and count them in [`FrameTapReceiver::dropped`].
    /// The receiver does not keep the transport alive. As with the transport's
    /// tap, stopping alone does not end it: drop it when observation is over.
    pub fn frames(&self, capacity: usize) -> Option<FrameTapReceiver> {
        self.is_running()
            .then(|| {
                self.frame_tap
                    .as_ref()
                    .map(|tap| tap.subscribe_with_capacity(capacity))
            })
            .flatten()
    }
}
