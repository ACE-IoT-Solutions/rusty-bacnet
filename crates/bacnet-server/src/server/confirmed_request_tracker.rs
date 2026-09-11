use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::request_peer::{canonical_requester, CanonicalRequester};
use bacnet_encoding::apdu::ConfirmedRequest;
use bacnet_encoding::npdu::NpduAddress;

/// Local resource policy for exact in-flight confirmed-request detection.
///
/// Clause 5.3.5.3 requires a server to discard a duplicate when it can detect
/// one, but does not mandate these bounds or exact-request discrimination. The
/// generic tracker only suppresses an exact request while its handler is
/// active. Completion releases the Invoke ID immediately so a client can reuse
/// it for a later transaction; service-specific durable idempotency policies
/// remain responsible for their own replay semantics.
const MAX_ENTRIES: usize = 256;
const MAX_TRACKED_SERVICE_REQUEST_BYTES: usize = 64 * 1024;

struct Entry {
    id: u64,
    requester: CanonicalRequester,
    invoke_id: u8,
    request: ConfirmedRequest,
}

#[derive(Default)]
struct TrackerState {
    next_id: u64,
    entries: VecDeque<Entry>,
}

/// Server-lifetime, bounded, exact confirmed-request duplicate tracker.
#[derive(Default)]
pub(super) struct ConfirmedRequestTracker {
    state: Mutex<TrackerState>,
}

pub(super) enum ConfirmedRequestAdmission {
    Duplicate,
    New(PendingConfirmedRequest),
}

/// RAII admission for one request that is pending handler completion.
///
/// Normal handler completion calls [`Self::complete`]. Cancellation or panic
/// drops an incomplete admission and removes its pending entry so a retry can
/// be serviced.
pub(super) struct PendingConfirmedRequest {
    tracker: Arc<ConfirmedRequestTracker>,
    id: Option<u64>,
    completed: bool,
}

impl ConfirmedRequestTracker {
    pub(super) fn begin(
        self: &Arc<Self>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        request: ConfirmedRequest,
    ) -> ConfirmedRequestAdmission {
        if request.service_request.len() > MAX_TRACKED_SERVICE_REQUEST_BYTES {
            return ConfirmedRequestAdmission::New(PendingConfirmedRequest::untracked(self));
        }

        let requester = canonical_requester(source_mac, source_network);
        let invoke_id = request.invoke_id;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.entries.iter().any(|entry| {
            entry.requester == requester && entry.invoke_id == invoke_id && entry.request == request
        }) {
            return ConfirmedRequestAdmission::Duplicate;
        }

        if state.entries.len() >= MAX_ENTRIES {
            // Every bounded slot is still executing. Detection is not safe
            // here, so Clause 5.3.5.3 permits normal untracked service.
            return ConfirmedRequestAdmission::New(PendingConfirmedRequest::untracked(self));
        }

        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        state.entries.push_back(Entry {
            id,
            requester,
            invoke_id,
            request,
        });
        ConfirmedRequestAdmission::New(PendingConfirmedRequest {
            tracker: Arc::clone(self),
            id: Some(id),
            completed: false,
        })
    }
}

impl PendingConfirmedRequest {
    fn untracked(tracker: &Arc<ConfirmedRequestTracker>) -> Self {
        Self {
            tracker: Arc::clone(tracker),
            id: None,
            completed: false,
        }
    }

    pub(super) fn complete(self) {
        let mut this = self;
        if let Some(id) = this.id {
            let mut state = this
                .tracker
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.entries.retain(|entry| entry.id != id);
        }
        this.completed = true;
    }
}

impl Drop for PendingConfirmedRequest {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        if let Some(id) = self.id {
            let mut state = self
                .tracker
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.entries.retain(|entry| entry.id != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;

    use bacnet_types::enums::ConfirmedServiceChoice;
    use bacnet_types::MacAddr;
    use bytes::Bytes;

    use super::*;

    fn request(invoke_id: u8, body: impl Into<Bytes>) -> ConfirmedRequest {
        ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: Some(4),
            max_apdu_length: 480,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            service_request: body.into(),
        }
    }

    fn routed(network: u16, mac: &[u8]) -> NpduAddress {
        NpduAddress {
            network,
            mac_address: MacAddr::from_slice(mac),
        }
    }

    fn expect_new(admission: ConfirmedRequestAdmission) -> PendingConfirmedRequest {
        match admission {
            ConfirmedRequestAdmission::New(pending) => pending,
            ConfirmedRequestAdmission::Duplicate => panic!("first request was a duplicate"),
        }
    }

    #[test]
    fn exact_request_is_duplicate_only_while_pending() {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let req = request(1, Bytes::from_static(b"request"));
        let pending = expect_new(tracker.begin(b"peer", None, req.clone()));
        assert!(matches!(
            tracker.begin(b"peer", None, req.clone()),
            ConfirmedRequestAdmission::Duplicate
        ));

        pending.complete();
        let reused = expect_new(tracker.begin(b"peer", None, req.clone()));
        assert!(matches!(
            tracker.begin(b"peer", None, req),
            ConfirmedRequestAdmission::Duplicate
        ));
        reused.complete();
    }

    #[test]
    fn completion_allows_exact_and_changed_invoke_reuse() {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let first = request(7, Bytes::from_static(b"one"));
        expect_new(tracker.begin(b"peer", None, first.clone())).complete();

        expect_new(tracker.begin(b"peer", None, first)).complete();
        expect_new(tracker.begin(b"peer", None, request(7, Bytes::from_static(b"two"))))
            .complete();
        let mut changed_service = request(7, Bytes::from_static(b"one"));
        changed_service.service_choice = ConfirmedServiceChoice::DELETE_OBJECT;
        assert!(matches!(
            tracker.begin(b"peer", None, changed_service),
            ConfirmedRequestAdmission::New(_)
        ));
    }

    #[test]
    fn canonical_routed_origin_ignores_router_and_peers_remain_independent() {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let req = request(2, Bytes::from_static(b"same"));
        let origin = routed(5, b"origin");
        let pending = expect_new(tracker.begin(b"router-a", Some(&origin), req.clone()));
        assert!(matches!(
            tracker.begin(b"router-b", Some(&origin), req.clone()),
            ConfirmedRequestAdmission::Duplicate
        ));
        assert!(matches!(
            tracker.begin(b"router-b", Some(&routed(6, b"origin")), req.clone()),
            ConfirmedRequestAdmission::New(_)
        ));
        assert!(matches!(
            tracker.begin(b"direct-a", None, req.clone()),
            ConfirmedRequestAdmission::New(_)
        ));
        assert!(matches!(
            tracker.begin(b"direct-b", None, req.clone()),
            ConfirmedRequestAdmission::New(_)
        ));
        pending.complete();

        let invalid = routed(0, b"claimed-origin");
        let invalid_pending = expect_new(tracker.begin(b"router-c", Some(&invalid), req.clone()));
        assert!(matches!(
            tracker.begin(b"router-d", Some(&invalid), req),
            ConfirmedRequestAdmission::New(_)
        ));
        invalid_pending.complete();
    }

    #[test]
    fn sequential_completion_reclaims_capacity_across_invoke_id_wrap() {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        for index in 0..512usize {
            let invoke_id = index as u8;
            let body = Bytes::from(vec![(index & 0xff) as u8, (index >> 8) as u8]);
            expect_new(tracker.begin(b"peer", None, request(invoke_id, body))).complete();
            assert!(tracker.state.lock().unwrap().entries.is_empty());
        }

        // The 513th request is byte-for-byte identical to the first request,
        // including its legally reused Invoke ID, and must still be admitted.
        expect_new(tracker.begin(b"peer", None, request(0, Bytes::from_static(&[0, 0]))))
            .complete();
    }

    #[test]
    fn all_pending_capacity_and_oversize_requests_fall_back_untracked() {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let mut pending = Vec::new();
        for index in 0..MAX_ENTRIES {
            pending.push(expect_new(tracker.begin(
                b"peer",
                None,
                request(4, Bytes::from(vec![index as u8, (index >> 8) as u8])),
            )));
        }
        assert!(matches!(
            tracker.begin(b"peer", None, request(4, Bytes::from_static(&[0, 0]))),
            ConfirmedRequestAdmission::Duplicate
        ));
        let fallback = expect_new(tracker.begin(
            b"peer",
            None,
            request(4, Bytes::from_static(b"fallback")),
        ));
        assert!(fallback.id.is_none());
        assert_eq!(tracker.state.lock().unwrap().entries.len(), MAX_ENTRIES);
        drop(fallback);
        assert!(matches!(
            tracker.begin(b"peer", None, request(4, Bytes::from_static(b"fallback"))),
            ConfirmedRequestAdmission::New(_)
        ));
        drop(pending);

        let oversized = expect_new(tracker.begin(
            b"peer",
            None,
            request(
                5,
                Bytes::from(vec![0; MAX_TRACKED_SERVICE_REQUEST_BYTES + 1]),
            ),
        ));
        assert!(oversized.id.is_none());
    }

    #[test]
    fn raii_drop_reclaims_cancelled_pending_and_restart_allows_service_again() {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let req = request(6, Bytes::from_static(b"cancelled"));
        let pending = expect_new(tracker.begin(b"peer", None, req.clone()));
        drop(pending);
        assert!(matches!(
            tracker.begin(b"peer", None, req.clone()),
            ConfirmedRequestAdmission::New(_)
        ));

        let restarted = Arc::new(ConfirmedRequestTracker::default());
        assert!(matches!(
            restarted.begin(b"peer", None, req),
            ConfirmedRequestAdmission::New(_)
        ));
    }

    #[test]
    fn concurrent_exact_admission_is_atomic() {
        const WORKERS: usize = 16;
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let start = Arc::new(Barrier::new(WORKERS + 1));
        let admitted = Arc::new(AtomicUsize::new(0));
        let finish = Arc::new(Barrier::new(WORKERS + 1));
        let mut workers = Vec::new();

        for _ in 0..WORKERS {
            let tracker = Arc::clone(&tracker);
            let start = Arc::clone(&start);
            let admitted = Arc::clone(&admitted);
            let finish = Arc::clone(&finish);
            workers.push(std::thread::spawn(move || {
                start.wait();
                let pending = match tracker.begin(
                    b"peer",
                    None,
                    request(8, Bytes::from_static(b"concurrent")),
                ) {
                    ConfirmedRequestAdmission::Duplicate => None,
                    ConfirmedRequestAdmission::New(pending) => {
                        admitted.fetch_add(1, Ordering::AcqRel);
                        Some(pending)
                    }
                };
                finish.wait();
                drop(pending);
            }));
        }

        start.wait();
        finish.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(admitted.load(Ordering::Acquire), 1);
    }
}
