//! The statistics engine against scripted lines, with hand-set times: no
//! port, no task, no clock.

use std::time::Duration;

use bacnet_mstp_codec::{FrameHeader, FrameType};

use super::script::{analyse, Damage, LineScript, ScriptedRing, TURNAROUND};
use super::stats::{
    octet_time, LineErrorKind, LineEvent, MonitorStats, NO_TOKEN_SILENCE, SILENCE_BUCKET_BOUNDS,
};

const BAUD: u32 = 38_400;
/// A request NPDU (control octet 0x04: a reply is expected).
const REQUEST: [u8; 3] = [0x01, 0x04, 0x10];
/// A reply NPDU.
const REPLY: [u8; 3] = [0x01, 0x00, 0x20];
/// A broadcast NPDU.
const ANNOUNCE: [u8; 3] = [0x01, 0x00, 0x30];

/// The issue's ring: masters 1 and 2 pass the token, station 2 runs with
/// Max_Master 3 so it never polls past 3, and station 5, a master the
/// ring never finds, answers the requests station 1 sends it.
fn acceptance_ring(holds: usize) -> ScriptedRing {
    let mut ring = ScriptedRing::new(BAUD).master(1, 127).master(2, 3);
    ring.rotate(holds, answer_from_5);
    ring
}

fn answer_from_5(line: &mut LineScript, holder: u8) {
    if holder == 1 {
        line.frame(FrameType::BACNET_DATA_EXPECTING_REPLY, 5, 1, &REQUEST)
            .frame(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 1, 5, &REPLY);
    }
}

fn analyse_ring(ring: &ScriptedRing) -> MonitorStats {
    analyse(BAUD, ring.line.chunks())
}

#[test]
fn ring_reports_present_stations_token_holders_and_max_master() {
    let stats = analyse_ring(&acceptance_ring(40));

    assert_eq!(stats.present_stations().collect::<Vec<_>>(), [1, 2, 5]);
    let five = stats.station(5).unwrap();
    assert_eq!(five.tokens_received, 0, "station 5 never gets the token");
    assert_eq!(five.tokens_passed, 0);
    assert!(!five.is_master());
    assert_eq!(five.frames_sent.data_not_expecting_reply, 20);
    assert_eq!(five.frames_received.data_expecting_reply, 20);

    let two = stats.station(2).unwrap();
    assert_eq!(two.max_master_inferred, Some(3));
    assert_eq!(two.next_station, Some(1));
    assert_eq!(two.tokens_received, 20);
    assert_eq!(two.tokens_passed, 20);
    // Its polls go to 3, then 0, then it passes without one, and again.
    assert_eq!(two.frames_sent.poll_for_master, 14);

    let one = stats.station(1).unwrap();
    // Station 1's successor is the next address, so it never polls.
    assert_eq!(one.max_master_inferred, None);
    assert_eq!(one.next_station, Some(2));
    assert!(one.is_master() && two.is_master());

    // Stations 0 and 3 are polled but never answer.
    assert!(!stats.station(0).unwrap().is_present());
    assert!(!stats.station(3).unwrap().is_present());
    assert_eq!(stats.token_passes, 40);
    assert_eq!(stats.lost_tokens, 0);
    assert_eq!(stats.errors, Default::default());
    assert_eq!(stats.frames.total(), 40 + 14 + 40);
}

#[test]
fn rotation_and_usage_delay_are_timed() {
    let stats = analyse_ring(&acceptance_ring(40));
    let one = stats.station(1).unwrap();
    // Twenty Tokens from station 1 give nineteen rotations.
    assert_eq!(one.token_rotation.count, 19);
    let shortest = one.token_rotation.min.unwrap();
    let longest = one.token_rotation.max.unwrap();
    assert!(shortest < longest, "{shortest:?} {longest:?}");
    assert!(one.token_rotation.mean().unwrap() > shortest);
    // Each station takes the token up a turnaround after it arrives;
    // nothing follows the last Token, to station 1.
    let two = stats.station(2).unwrap();
    assert_eq!(two.token_usage_delay.count, 20);
    assert_eq!(two.token_usage_delay.max, Some(TURNAROUND));
    assert_eq!(one.token_usage_delay.count, 19);
    assert_eq!(one.token_usage_delay.min, Some(TURNAROUND));
}

#[test]
fn crc_errors_are_counted_exactly() {
    const HEADER_ERRORS: usize = 3;
    const DATA_ERRORS: usize = 5;
    let mut ring = ScriptedRing::new(BAUD).master(1, 127).master(2, 3);
    let mut sent = 0;
    ring.rotate(40, |line, holder| {
        if holder != 2 {
            return;
        }
        let damage = match sent {
            n if n < HEADER_ERRORS => Some(Damage::HeaderCrc),
            n if n < HEADER_ERRORS + DATA_ERRORS => Some(Damage::DataCrc),
            _ => None,
        };
        sent += 1;
        match damage {
            Some(damage) => line.damaged(
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                255,
                2,
                &ANNOUNCE,
                damage,
            ),
            None => line.frame(
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                255,
                2,
                &ANNOUNCE,
            ),
        };
    });
    let stats = analyse_ring(&ring);
    assert_eq!(stats.errors.header_crc, HEADER_ERRORS as u64);
    assert_eq!(stats.errors.data_crc, DATA_ERRORS as u64);
    assert_eq!(stats.errors.framing, 0);
    assert_eq!(stats.errors.frame_aborts, 0);
    let two = stats.station(2).unwrap();
    assert_eq!(
        two.frames_sent.data_not_expecting_reply,
        (20 - HEADER_ERRORS - DATA_ERRORS) as u64
    );
    // The damage cost nothing else.
    assert_eq!(stats.token_passes, 40);
    assert_eq!(two.max_master_inferred, Some(3));
}

#[test]
fn one_lost_token_is_one_event() {
    let mut ring = acceptance_ring(20);
    ring.lose_token(Duration::from_millis(600));
    ring.rotate(20, answer_from_5);
    let stats = analyse_ring(&ring);
    assert_eq!(stats.lost_tokens, 1);
    let lost = stats.last_lost_token.unwrap();
    assert_eq!(lost.regenerated_by, Some(1));
    assert!(lost.silence >= Duration::from_millis(600), "{lost:?}");
    assert!(lost.silence < Duration::from_millis(602), "{lost:?}");
    assert_eq!(stats.silences.counts[SILENCE_BUCKET_BOUNDS.len()], 1);
    assert_eq!(stats.silences.longest, Some(lost.silence));
    // The generation poll gives station 1 a Max_Master estimate too.
    assert_eq!(stats.station(1).unwrap().max_master_inferred, Some(2));
    assert_eq!(stats.station(2).unwrap().max_master_inferred, Some(3));
}

#[test]
fn a_silence_just_short_of_no_token_is_not_a_lost_token() {
    let mut ring = acceptance_ring(4);
    ring.line.idle(NO_TOKEN_SILENCE - Duration::from_millis(2));
    ring.rotate(4, answer_from_5);
    let stats = analyse_ring(&ring);
    assert_eq!(stats.lost_tokens, 0);
    assert!(stats.silences.longest.unwrap() < NO_TOKEN_SILENCE);
}

#[test]
fn a_coarse_adapter_changes_times_not_counts() {
    let mut ring = acceptance_ring(30);
    ring.lose_token(Duration::from_millis(700));
    ring.rotate(30, answer_from_5);
    let fine = analyse_ring(&ring);
    let coarse = analyse(BAUD, &ring.line.rechunked(Duration::from_millis(16)));
    assert!(coarse.timing.max_octets_per_read > fine.timing.max_octets_per_read);
    assert!(coarse.timing.reads < fine.timing.reads);
    assert_eq!(coarse.octets, fine.octets);
    assert_eq!(coarse.frames, fine.frames);
    assert_eq!(coarse.lost_tokens, 1);
    for (address, station) in &fine.stations {
        let other = coarse.station(*address).unwrap();
        assert_eq!(other.frames_sent, station.frames_sent, "station {address}");
        assert_eq!(other.tokens_received, station.tokens_received);
        assert_eq!(other.max_master_inferred, station.max_master_inferred);
    }
}

#[test]
fn a_frame_cut_off_by_silence_is_a_frame_abort() {
    let mut line = LineScript::new(BAUD);
    line.frame(FrameType::TOKEN, 2, 1, &[]);
    // Half a header, then nothing for 200 ms.
    line.raw(&[0x55, 0xFF, 0x00, 0x03])
        .idle(Duration::from_millis(200));
    line.frame(FrameType::TOKEN, 3, 2, &[]);
    let stats = analyse(BAUD, line.chunks());
    assert_eq!(stats.errors.frame_aborts, 1);
    assert_eq!(stats.frames.token, 2);
    assert_eq!(stats.station(2).unwrap().tokens_passed, 1);
}

#[test]
fn a_token_sent_twice_is_a_retry() {
    let mut line = LineScript::new(BAUD);
    line.frame(FrameType::TOKEN, 2, 1, &[])
        .idle(Duration::from_millis(20))
        .frame(FrameType::TOKEN, 2, 1, &[])
        .frame(FrameType::POLL_FOR_MASTER, 3, 2, &[]);
    let stats = analyse(BAUD, line.chunks());
    let one = stats.station(1).unwrap();
    assert_eq!(one.token_retries, 1);
    assert_eq!(one.tokens_passed, 2);
    assert_eq!(stats.station(2).unwrap().tokens_received, 2);
}

#[test]
fn cobs_frames_are_decoded_and_counted() {
    let npdu: Vec<u8> = (0..600u16).map(|value| value as u8).collect();
    let mut line = LineScript::new(BAUD);
    line.frame(
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        255,
        7,
        &npdu,
    )
    .damaged(
        FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
        4,
        7,
        &npdu,
        Damage::DataCrc,
    );
    let stats = analyse(BAUD, line.chunks());
    assert_eq!(stats.cobs_frames, 1);
    assert_eq!(stats.frames.extended_data_not_expecting_reply, 1);
    assert_eq!(stats.errors.data_crc, 1);
}

#[test]
fn noise_between_frames_is_counted_and_harmless() {
    let mut line = LineScript::new(BAUD);
    line.raw(&[0x00, 0x12, 0x34])
        .frame(FrameType::TOKEN, 2, 1, &[])
        .raw(&[0xFF, 0xFE])
        .frame(FrameType::TOKEN, 1, 2, &[]);
    let stats = analyse(BAUD, line.chunks());
    assert_eq!(stats.errors.noise_octets, 5);
    assert_eq!(stats.token_passes, 2);
}

#[test]
fn events_fed_by_hand() {
    let mut stats = MonitorStats::new(BAUD, Duration::from_millis(16));
    let token = |destination, source| LineEvent::Frame {
        header: FrameHeader::new(FrameType::TOKEN, destination, source),
        wire_length: 8,
    };
    stats.record(Duration::from_millis(10), &token(2, 1));
    stats.record(Duration::from_millis(15), &token(1, 2));
    stats.record(Duration::from_millis(20), &token(2, 1));
    stats.record(
        Duration::from_millis(25),
        &LineEvent::Error {
            kind: LineErrorKind::Framing,
            destination: Some(3),
            octets: 8,
        },
    );
    assert_eq!(stats.timing.resolution, Duration::from_millis(16));
    assert_eq!(stats.timing.octet_time, octet_time(BAUD));
    assert_eq!(stats.errors.framing, 1);
    let one = stats.station(1).unwrap();
    assert_eq!(one.token_rotation.count, 1);
    assert_eq!(one.token_rotation.last, Some(Duration::from_millis(10)));
    // Usage delay: from the end of the Token to the start of the answer.
    let eight_octets = octet_time(BAUD) * 8;
    assert_eq!(
        stats.station(2).unwrap().token_usage_delay.last,
        Some(Duration::from_millis(5) - eight_octets)
    );
    // Three silences, all in the first few buckets.
    assert_eq!(stats.silences.counts.iter().sum::<u64>(), 3);
}

#[test]
fn octet_time_is_ten_bit_times() {
    assert_eq!(octet_time(9_600), Duration::from_nanos(1_041_666));
    assert_eq!(octet_time(76_800), Duration::from_nanos(130_208));
    assert_eq!(octet_time(0), Duration::ZERO);
}
