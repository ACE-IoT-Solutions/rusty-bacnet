//! Opt-in receiver reservation for aggregate runtimes.

use super::*;

impl<T: TransportPort> ClientBuilder<T> {
    /// Reserve startup notification receivers before dispatch begins.
    pub fn reserve_initial_observers(mut self) -> Self {
        self.options.reserve_initial_observers = true;
        self
    }
}

impl BipClientBuilder {
    /// Reserve startup notification receivers before dispatch begins.
    pub fn reserve_initial_observers(mut self) -> Self {
        self.options.reserve_initial_observers = true;
        self
    }
}
