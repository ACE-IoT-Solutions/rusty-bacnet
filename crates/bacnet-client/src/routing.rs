//! Client-side router table for auto-routing to remote BACnet networks.
//!
//! Populated from I-Am-Router-To-Network messages received by the dispatch task.

use std::collections::HashMap;
use std::time::Instant;

use bacnet_types::MacAddr;

/// Entry in the client router table.
#[derive(Debug, Clone)]
struct RouteEntry {
    router_mac: MacAddr,
    last_seen: Instant,
}

/// A simple mapping from BACnet network number to the router MAC that
/// can reach that network.
#[derive(Debug)]
pub struct ClientRouterTable {
    routes: HashMap<u16, RouteEntry>,
}

impl ClientRouterTable {
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
        }
    }

    /// Record that `router_mac` can route to `network`.
    pub fn insert(&mut self, network: u16, router_mac: MacAddr) {
        self.routes.insert(
            network,
            RouteEntry {
                router_mac,
                last_seen: Instant::now(),
            },
        );
    }

    /// Look up the router MAC for a given network number.
    pub fn lookup(&self, network: u16) -> Option<MacAddr> {
        self.routes.get(&network).map(|e| e.router_mac.clone())
    }

    /// Remove stale entries older than `max_age`.
    pub fn purge_stale(&mut self, max_age: std::time::Duration) {
        let now = Instant::now();
        self.routes
            .retain(|_, entry| now.duration_since(entry.last_seen) < max_age);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_lookup() {
        let mut table = ClientRouterTable::new();
        let mac = MacAddr::from_slice(&[10, 0, 0, 1, 0xBA, 0xC0]);
        table.insert(1001, mac.clone());
        assert_eq!(table.lookup(1001), Some(mac));
        assert_eq!(table.lookup(1002), None);
    }

    #[test]
    fn overwrite_updates_entry() {
        let mut table = ClientRouterTable::new();
        let mac1 = MacAddr::from_slice(&[10, 0, 0, 1, 0xBA, 0xC0]);
        let mac2 = MacAddr::from_slice(&[10, 0, 0, 2, 0xBA, 0xC0]);
        table.insert(1001, mac1);
        table.insert(1001, mac2.clone());
        assert_eq!(table.lookup(1001), Some(mac2));
    }
}
