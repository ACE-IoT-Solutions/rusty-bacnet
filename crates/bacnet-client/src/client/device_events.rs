use super::*;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Get a receiver for device discovery events. Each call returns a new
    /// independent receiver.
    ///
    /// Events are notification-only; `discovered_devices()` remains the
    /// authoritative snapshot of the current discovery table.
    pub fn device_events(&self) -> broadcast::Receiver<DeviceEvent> {
        self.device_tx.subscribe()
    }

    /// Get a receiver for Device-instance collision notifications. Each call
    /// returns a new independent receiver.
    ///
    /// Delivery is notification-only and best-effort: receivers can lag and
    /// missed notifications do not affect collision handling. The `retained`
    /// snapshot identifies the discovery-table row that remains authoritative;
    /// the conflicting `incoming` snapshot is not installed in the table.
    pub fn device_collision_events(&self) -> broadcast::Receiver<DeviceCollisionEvent> {
        self.device_collision_tx.subscribe()
    }

    /// Subscribe to raw I-Am observations.
    ///
    /// Every valid I-Am is broadcast, including duplicates. Receivers that
    /// fall behind observe Tokio's `RecvError::Lagged`; packet dispatch never
    /// waits for a consumer.
    pub fn iam_events(&self) -> broadcast::Receiver<IAmEvent> {
        self.iam_tx.subscribe()
    }

    /// Take the receiver reserved before the client's dispatch task started.
    ///
    /// Intended for composed runtimes that install their consumer immediately
    /// after construction. Returns `None` after the first call.
    pub fn take_initial_iam_receiver(&mut self) -> Option<broadcast::Receiver<IAmEvent>> {
        self.initial_iam_rx.take()
    }
}
