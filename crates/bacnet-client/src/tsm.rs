//! Transaction State Machine (TSM) per ASHRAE 135-2020 Clause 5.4.
//!
//! Tracks in-flight confirmed requests. Each request gets a unique invoke_id
//! (0-255) scoped per destination MAC. Responses are delivered via oneshot channels.

use bacnet_types::MacAddr;
use bytes::Bytes;
use std::collections::HashMap;
use tokio::sync::oneshot;
use tracing::{debug, warn};

/// TSM configuration.
#[derive(Debug, Clone)]
pub struct TsmConfig {
    /// APDU timeout in milliseconds (default 6000).
    pub apdu_timeout_ms: u64,
    /// APDU segment timeout in milliseconds (default = apdu_timeout_ms).
    pub apdu_segment_timeout_ms: u64,
    /// Number of APDU retries (default 3).
    pub apdu_retries: u8,
}

impl Default for TsmConfig {
    fn default() -> Self {
        Self {
            apdu_timeout_ms: 6000,
            apdu_segment_timeout_ms: 6000,
            apdu_retries: 3,
        }
    }
}

/// Response types that complete a transaction.
#[derive(Debug)]
pub enum TsmResponse {
    /// SimpleACK — confirmed service completed with no return data.
    SimpleAck,
    /// ComplexACK — confirmed service returned data.
    ComplexAck { service_data: Bytes },
    /// Error PDU.
    Error { class: u32, code: u32 },
    /// Reject PDU.
    Reject { reason: u8 },
    /// Abort PDU.
    Abort { reason: u8 },
}

/// Invoke ID allocator scoped to a single destination MAC.
struct InvokeIdAllocator {
    next_id: u8,
    in_use: [bool; 256],
}

impl InvokeIdAllocator {
    fn new() -> Self {
        Self {
            next_id: 0,
            in_use: [false; 256],
        }
    }

    fn allocate(&mut self) -> Option<u8> {
        let start = self.next_id;
        loop {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            if !self.in_use[id as usize] {
                self.in_use[id as usize] = true;
                return Some(id);
            }
            if self.next_id == start {
                return None;
            }
        }
    }

    fn release(&mut self, id: u8) {
        self.in_use[id as usize] = false;
    }

    fn all_free(&self) -> bool {
        !self.in_use.iter().any(|&used| used)
    }
}

/// Maximum number of distinct destination MACs tracked by the TSM.
/// Prevents unbounded memory growth from spoofed source addresses.
const MAX_TSM_DESTINATIONS: usize = 1024;

/// Transaction State Machine.
///
/// Tracks pending confirmed requests and correlates responses by
/// `(destination_mac, invoke_id)`.
pub struct Tsm {
    config: TsmConfig,
    allocators: HashMap<MacAddr, InvokeIdAllocator>,
    pending: HashMap<(MacAddr, u8), oneshot::Sender<TsmResponse>>,
}

impl Tsm {
    pub fn new(config: TsmConfig) -> Self {
        Self {
            config,
            allocators: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    pub fn config(&self) -> &TsmConfig {
        &self.config
    }

    /// Allocate an invoke ID for the given destination MAC.
    /// Returns `None` if all 256 IDs are in use for this destination,
    /// or if the maximum number of tracked destinations has been reached.
    pub fn allocate_invoke_id(&mut self, destination_mac: &[u8]) -> Option<u8> {
        let key = MacAddr::from_slice(destination_mac);
        if !self.allocators.contains_key(&key) && self.allocators.len() >= MAX_TSM_DESTINATIONS {
            return None;
        }
        let allocator = self
            .allocators
            .entry(key)
            .or_insert_with(InvokeIdAllocator::new);
        allocator.allocate()
    }

    /// Release an invoke ID back to the pool for the given destination.
    /// Removes the allocator entry if all IDs are now free (prevents unbounded growth).
    pub fn release_invoke_id(&mut self, destination_mac: &[u8], invoke_id: u8) {
        let key = MacAddr::from_slice(destination_mac);
        if let Some(allocator) = self.allocators.get_mut(&key) {
            allocator.release(invoke_id);
            if allocator.all_free() {
                self.allocators.remove(&key);
            }
        }
    }

    /// Register a pending transaction. Returns a receiver that will deliver
    /// the response when it arrives.
    pub fn register_transaction(
        &mut self,
        destination_mac: MacAddr,
        invoke_id: u8,
    ) -> oneshot::Receiver<TsmResponse> {
        let (tx, rx) = oneshot::channel();
        debug_assert!(
            !self
                .pending
                .contains_key(&(destination_mac.clone(), invoke_id)),
            "duplicate TSM registration for invoke_id {}",
            invoke_id
        );
        self.pending.insert((destination_mac, invoke_id), tx);
        rx
    }

    /// Deliver a response to a pending transaction. Returns `true` if found.
    ///
    /// First attempts an exact match on `(source_mac, invoke_id)`. If that
    /// fails, falls back to matching by `invoke_id` alone — this handles
    /// cross-subnet scenarios where the response may arrive from a different
    /// transport address than the request was sent to (e.g., BBMD relay,
    /// asymmetric routing).
    pub fn complete_transaction(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        response: TsmResponse,
    ) -> bool {
        let key = (MacAddr::from_slice(source_mac), invoke_id);

        // Fast path: exact (source_mac, invoke_id) match.
        if let Some(tx) = self.pending.remove(&key) {
            self.release_invoke_id(source_mac, invoke_id);
            let _ = tx.send(response);
            return true;
        }

        // Fallback: match by invoke_id alone. This catches cross-subnet
        // routed responses where the transport source address differs from
        // the original destination (e.g., response routed through a BBMD
        // or arriving from a different IP path).
        let mut fallback_key = None;
        for (pending_key, _) in self.pending.iter() {
            if pending_key.1 == invoke_id {
                if fallback_key.is_some() {
                    // Multiple pending transactions share this invoke_id
                    // (different destinations) — ambiguous, can't safely match.
                    debug!(
                        invoke_id,
                        source_mac = ?MacAddr::from_slice(source_mac),
                        "TSM fallback match aborted: multiple pending transactions with invoke_id"
                    );
                    fallback_key = None;
                    break;
                }
                fallback_key = Some(pending_key.clone());
            }
        }

        if let Some(fk) = fallback_key {
            warn!(
                invoke_id,
                response_source = ?MacAddr::from_slice(source_mac),
                expected_source = ?fk.0,
                "TSM cross-subnet fallback: response arrived from unexpected address, \
                 matched by invoke_id alone"
            );
            if let Some(tx) = self.pending.remove(&fk) {
                self.release_invoke_id(&fk.0, invoke_id);
                let _ = tx.send(response);
                return true;
            }
        }

        if !self.pending.is_empty() {
            warn!(
                invoke_id,
                source_mac = ?MacAddr::from_slice(source_mac),
                pending_count = self.pending.len(),
                pending_keys = ?self.pending.keys().collect::<Vec<_>>(),
                "TSM: no matching transaction for response"
            );
        }

        false
    }

    /// Cancel a pending transaction. Returns `true` if found.
    pub fn cancel_transaction(&mut self, destination_mac: &[u8], invoke_id: u8) -> bool {
        let key = (MacAddr::from_slice(destination_mac), invoke_id);
        if self.pending.remove(&key).is_some() {
            self.release_invoke_id(destination_mac, invoke_id);
            true
        } else {
            false
        }
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_invoke_id_sequential() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = [127, 0, 0, 1, 0xBA, 0xC0];
        let id1 = tsm.allocate_invoke_id(&mac);
        let id2 = tsm.allocate_invoke_id(&mac);
        assert_eq!(id1, Some(0));
        assert_eq!(id2, Some(1));
    }

    #[test]
    fn allocate_invoke_id_per_destination() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac_a = [10, 0, 0, 1, 0xBA, 0xC0];
        let mac_b = [10, 0, 0, 2, 0xBA, 0xC0];
        let id_a = tsm.allocate_invoke_id(&mac_a);
        let id_b = tsm.allocate_invoke_id(&mac_b);
        assert_eq!(id_a, Some(0));
        assert_eq!(id_b, Some(0));
    }

    #[test]
    fn allocate_invoke_id_wraps() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = [127, 0, 0, 1, 0xBA, 0xC0];
        for i in 0..256 {
            assert_eq!(tsm.allocate_invoke_id(&mac), Some(i as u8));
        }
        assert_eq!(tsm.allocate_invoke_id(&mac), None);
    }

    #[test]
    fn release_makes_id_available() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = [127, 0, 0, 1, 0xBA, 0xC0];
        let id0 = tsm.allocate_invoke_id(&mac).unwrap();
        let id1 = tsm.allocate_invoke_id(&mac).unwrap();
        assert_eq!(id0, 0);
        assert_eq!(id1, 1);
        tsm.release_invoke_id(&mac, id0);
        let id2 = tsm.allocate_invoke_id(&mac).unwrap();
        assert_eq!(id2, 2);
        tsm.release_invoke_id(&mac, id1);
        tsm.release_invoke_id(&mac, id2);
        let id3 = tsm.allocate_invoke_id(&mac).unwrap();
        assert_eq!(id3, 0);
    }

    #[tokio::test]
    async fn register_and_complete_transaction() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();

        let rx = tsm.register_transaction(mac.clone(), invoke_id);

        let response = TsmResponse::ComplexAck {
            service_data: Bytes::from_static(&[0xDE, 0xAD]),
        };
        let completed = tsm.complete_transaction(&mac, invoke_id, response);
        assert!(completed);

        let result = rx.await.unwrap();
        match result {
            TsmResponse::ComplexAck { service_data } => {
                assert_eq!(service_data, vec![0xDE, 0xAD]);
            }
            _ => panic!("Expected ComplexAck"),
        }
    }

    #[tokio::test]
    async fn complete_unknown_transaction_returns_false() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
        let completed = tsm.complete_transaction(&mac, 42, TsmResponse::SimpleAck);
        assert!(!completed);
    }

    #[tokio::test]
    async fn fallback_match_by_invoke_id_alone() {
        // Simulate cross-subnet scenario: request sent to router A,
        // response arrives from a different address (e.g., BBMD relay).
        let mut tsm = Tsm::new(TsmConfig::default());
        let router_mac = [10, 2, 0, 10, 0xBA, 0xC0]; // 10.2.0.10:47808
        let invoke_id = tsm.allocate_invoke_id(&router_mac).unwrap();
        let rx = tsm.register_transaction(MacAddr::from_slice(&router_mac), invoke_id);

        // Response arrives from a different address (e.g., BBMD at 10.1.0.2)
        let bbmd_mac = [10, 1, 0, 2, 0xBA, 0xC0]; // 10.1.0.2:47808
        let completed = tsm.complete_transaction(
            &bbmd_mac,
            invoke_id,
            TsmResponse::ComplexAck {
                service_data: Bytes::from_static(&[0xBE, 0xEF]),
            },
        );
        assert!(completed, "fallback match should succeed");

        let result = rx.await.unwrap();
        match result {
            TsmResponse::ComplexAck { service_data } => {
                assert_eq!(service_data, vec![0xBE, 0xEF]);
            }
            _ => panic!("Expected ComplexAck"),
        }

        // Verify invoke_id was released (allocator cleaned up)
        assert_eq!(tsm.pending_count(), 0);
    }

    #[tokio::test]
    async fn fallback_match_ambiguous_skipped() {
        // When multiple pending transactions share the same invoke_id
        // (different destinations), fallback should NOT match.
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac_a = [10, 0, 0, 1, 0xBA, 0xC0];
        let mac_b = [10, 0, 0, 2, 0xBA, 0xC0];

        let id_a = tsm.allocate_invoke_id(&mac_a).unwrap();
        let id_b = tsm.allocate_invoke_id(&mac_b).unwrap();
        // Both get invoke_id 0 (per-destination allocation)
        assert_eq!(id_a, id_b);

        let _rx_a = tsm.register_transaction(MacAddr::from_slice(&mac_a), id_a);
        let _rx_b = tsm.register_transaction(MacAddr::from_slice(&mac_b), id_b);

        // Response from an unknown source — should NOT match because ambiguous
        let unknown_mac = [10, 0, 0, 99, 0xBA, 0xC0];
        let completed = tsm.complete_transaction(&unknown_mac, 0, TsmResponse::SimpleAck);
        assert!(!completed, "ambiguous fallback should not match");
        assert_eq!(tsm.pending_count(), 2);
    }

    #[test]
    fn cancel_transaction() {
        let mut tsm = Tsm::new(TsmConfig::default());
        let mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]);
        let invoke_id = tsm.allocate_invoke_id(&mac).unwrap();
        let _rx = tsm.register_transaction(mac.clone(), invoke_id);
        assert_eq!(tsm.pending_count(), 1);

        let cancelled = tsm.cancel_transaction(&mac, invoke_id);
        assert!(cancelled);
        assert_eq!(tsm.pending_count(), 0);
    }
}
