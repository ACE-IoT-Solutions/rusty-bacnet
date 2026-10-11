#![cfg(feature = "heapless")]
#![doc = "Integration tests for reserved MS/TP network-control capacity."]

use bacnet_mstp_codec::{decode_frame, Frame, FrameType, MAX_STANDARD_MPDU_DATA};
use bacnet_mstp_core::{
    Action, Direction, HeaplessQueues, Instant, MasterCore, MstpCoreConfig, NetworkPriority,
    TransmitId,
};

type Queues = HeaplessQueues<1, 0, MAX_STANDARD_MPDU_DATA, 1>;

fn config() -> MstpCoreConfig {
    MstpCoreConfig {
        station: 1,
        max_master: 2,
        max_info_frames: 3,
        ..MstpCoreConfig::default()
    }
}

fn take_transmit(core: &mut MasterCore<'_, Queues>, now: Instant) -> (TransmitId, u8) {
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    let (id, first_byte) = match core.poll_action() {
        Action::Transmit { id, frame } => {
            let (decoded, _) = decode_frame(frame).unwrap();
            (id, decoded.data[0])
        }
        other => panic!("expected transmit, got {other:?}"),
    };
    core.on_transmit_started(id, now).unwrap();
    (id, first_byte)
}

fn complete_transmit(core: &mut MasterCore<'_, Queues>, id: TransmitId, now: Instant) {
    core.on_transmit_complete(id, now).unwrap();
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
}

#[test]
fn control_queue_is_serviced_before_data_without_starving_data() {
    let mut rx = [0; 1024];
    let mut core = MasterCore::new(config(), Queues::new(), &mut rx, Instant::ZERO).unwrap();
    core.enqueue_npdu(2, &[0x10, 0], NetworkPriority::Normal, Instant::ZERO)
        .unwrap();
    core.enqueue_network_control_npdu(2, &[0x20, 0], NetworkPriority::Urgent, Instant::ZERO)
        .unwrap();
    assert_eq!(core.counters().unicast.depth, 1);
    assert_eq!(core.counters().network_control.depth, 1);

    core.on_validated_frame(
        Frame::new(FrameType::Token, 1, 2, &[]),
        Instant::from_micros(1),
    )
    .unwrap();
    core.on_deadline(Instant::from_micros(10_000)).unwrap();

    let (control_id, first_byte) = take_transmit(&mut core, Instant::from_micros(10_000));
    assert_eq!(first_byte, 0x20);
    complete_transmit(&mut core, control_id, Instant::from_micros(11_000));

    core.on_deadline(Instant::from_micros(16_000)).unwrap();
    let (data_id, second_byte) = take_transmit(&mut core, Instant::from_micros(16_000));
    assert_eq!(second_byte, 0x10);
    complete_transmit(&mut core, data_id, Instant::from_micros(17_000));
    assert_eq!(core.counters().network_control.depth, 0);
    assert_eq!(core.counters().unicast.depth, 0);
}
