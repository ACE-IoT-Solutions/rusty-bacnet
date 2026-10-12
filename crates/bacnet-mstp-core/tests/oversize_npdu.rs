//! An NPDU too long for a standard frame never holds the token: it is
//! refused at the queue, or dropped and counted when it gets in anyway.

#![cfg(feature = "heapless")]

use bacnet_mstp_codec::{decode_frame, Frame, FrameHeader, FrameType};
use bacnet_mstp_core::{
    Action, DequeuedNpdu, Direction, HeaplessQueues, Instant, MasterCore, MasterState,
    MstpCoreConfig, NetworkPriority, QueueCounters, QueueItemInfo, QueueKind, QueueRejectReason,
    QueueStorage,
};

/// Queues whose slots hold more than a standard frame's data.
type BigQueues = HeaplessQueues<4, 4, 1500>;
type Actions = bacnet_mstp_core::ActionBuffers;

fn config() -> MstpCoreConfig {
    MstpCoreConfig {
        station: 1,
        max_master: 2,
        ..MstpCoreConfig::default()
    }
}

/// Take the token and return the header of the frame it leads to.
fn next_frame_after_token<Q: QueueStorage>(core: &mut MasterCore<'_, Q>) -> FrameHeader {
    core.on_validated_frame(Frame::new(FrameType::Token, 1, 2, &[]), Instant::ZERO)
        .unwrap();
    let due = core.next_deadline().unwrap();
    assert!(due > Instant::ZERO, "the next deadline is in the future");
    core.on_deadline(due).unwrap();
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    match core.poll_action() {
        Action::Transmit { frame, .. } => decode_frame(frame).unwrap().0.header,
        other => panic!("expected transmit, got {other:?}"),
    }
}

#[test]
fn enqueue_refuses_an_npdu_longer_than_a_standard_frame() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        BigQueues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    for destination in [2, 0xFF] {
        assert_eq!(
            core.enqueue_npdu(
                destination,
                &[0; 502],
                NetworkPriority::Normal,
                Instant::ZERO
            ),
            Err(QueueRejectReason::Oversize)
        );
    }
    let counters = core.counters();
    assert_eq!(counters.unicast.drops_oversize, 1);
    assert_eq!(counters.broadcast.drops_oversize, 1);
    assert_eq!(counters.unicast.depth, 0);
}

#[test]
fn an_oversize_npdu_already_queued_is_dropped_and_the_token_moves_on() {
    // Queued straight into the storage before the core owns it.
    let mut queues = BigQueues::new();
    queues
        .enqueue_npdu(2, &[0; 600], NetworkPriority::Normal, 0, None)
        .unwrap();
    queues
        .enqueue_npdu(2, &[1, 0], NetworkPriority::Normal, 0, None)
        .unwrap();
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(config(), queues, &mut rx, &mut actions, Instant::ZERO).unwrap();
    let header = next_frame_after_token(&mut core);
    assert_eq!(
        (header.frame_type, header.destination),
        (FrameType::BACnetDataNotExpectingReply, 2)
    );
    assert_eq!(core.counters().unicast.drops_oversize, 1);

    // With nothing else queued the token use simply ends: here it polls for
    // a successor.
    let mut queues = BigQueues::new();
    queues
        .enqueue_npdu(2, &[0; 600], NetworkPriority::Normal, 0, None)
        .unwrap();
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(config(), queues, &mut rx, &mut actions, Instant::ZERO).unwrap();
    let header = next_frame_after_token(&mut core);
    assert_eq!(header.frame_type, FrameType::PollForMaster);
    assert_eq!(core.state(), MasterState::PollForMaster);
    assert_eq!(core.counters().unicast.drops_oversize, 1);
}

/// A queue storage that reports more octets than it copied.
struct Overstating(HeaplessQueues<4, 4>);

impl QueueStorage for Overstating {
    fn set_starvation_limit(&mut self, limit: u8) {
        self.0.set_starvation_limit(limit);
    }

    fn max_npdu_length(&self) -> usize {
        self.0.max_npdu_length()
    }

    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: u64,
        expires_at: Option<u64>,
    ) -> Result<(), QueueRejectReason> {
        self.0
            .enqueue_npdu(destination, payload, priority, now, expires_at)
    }

    fn expire(&mut self, now: u64) -> usize {
        self.0.expire(now)
    }

    fn peek(&self, now: u64) -> Option<QueueItemInfo> {
        self.0.peek(now)
    }

    fn peek_kind(&self, kind: QueueKind, now: u64) -> Option<QueueItemInfo> {
        self.0.peek_kind(kind, now)
    }

    fn dequeue_kind_into(
        &mut self,
        kind: QueueKind,
        now: u64,
        scratch: &mut [u8],
    ) -> Result<Option<DequeuedNpdu>, QueueRejectReason> {
        let item = self.0.dequeue_kind_into(kind, now, scratch)?;
        Ok(item.map(|item| DequeuedNpdu { len: 600, ..item }))
    }

    fn depth(&self, kind: QueueKind) -> usize {
        self.0.depth(kind)
    }

    fn stats(&self, kind: QueueKind) -> QueueCounters {
        self.0.stats(kind)
    }
}

#[test]
fn a_queue_overstating_an_npdu_length_cannot_hold_the_token() {
    let mut rx = [0; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::new(
        config(),
        Overstating(HeaplessQueues::new()),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    core.enqueue_npdu(2, &[1, 0], NetworkPriority::Normal, Instant::ZERO)
        .unwrap();
    let header = next_frame_after_token(&mut core);
    assert_eq!(header.frame_type, FrameType::PollForMaster);
    assert_eq!(core.counters().unicast.drops_oversize, 1);
}
