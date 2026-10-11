//! Scenario (g): Test_Request and Test_Response.

mod common;

use bacnet_mstp_codec::{FrameType, MAX_STANDARD_MPDU_DATA};
use bacnet_mstp_core::Duration;
use bacnet_mstp_sim::{MasterSpec, ScriptSend, SeenFrame, Sim, SimConfig};

use common::*;

/// The scripted station's reaction delay: just over Tturnaround at 38400
/// baud.
const PAUSE: Duration = Duration::from_micros(1_100);

/// Master 0 alone with a scripted station 5 that joins the ring by answering
/// polls and, each time it holds the token, sends what `on_token` returns
/// and passes the token back to 0 when that is answered or after 40 ms.
fn with_tester(mut on_token: impl FnMut(u32) -> Vec<ScriptSend> + 'static) -> Sim {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(MasterSpec::new(0, 7, BAUD));
    let mut tokens = 0;
    sim.add_scripted(5, move |frame: &SeenFrame, sends: &mut Vec<ScriptSend>| {
        if frame.destination != 5 {
            return;
        }
        match frame.frame_type {
            FrameType::PollForMaster => sends.push(ScriptSend::frame(
                PAUSE,
                FrameType::ReplyToPollForMaster,
                frame.source,
                5,
                &[],
            )),
            FrameType::Token => {
                let sent = on_token(tokens);
                tokens += 1;
                if sent.is_empty() {
                    sends.push(ScriptSend::frame(PAUSE, FrameType::Token, 0, 5, &[]));
                } else {
                    sends.extend(sent);
                }
            }
            FrameType::TestResponse => {
                sends.push(ScriptSend::frame(PAUSE, FrameType::Token, 0, 5, &[]));
            }
            _ => {}
        }
    });
    sim
}

/// Clauses 9.5.6.2 (ReceivedDataNeedingReply) and 9.5.6.9 (Reply).
///
/// A Test_Request addressed to a master is answered by its MS/TP layer, not
/// its application: a Test_Response carrying the same data goes back after
/// Tturnaround, for empty data, one octet and the largest standard data
/// field alike.
#[test]
fn a_test_request_is_echoed_by_its_target() {
    let payloads: Vec<Vec<u8>> = vec![
        Vec::new(),
        vec![0x42],
        (0..MAX_STANDARD_MPDU_DATA)
            .map(|index| index as u8)
            .collect(),
    ];
    let script_payloads = payloads.clone();
    let mut sim = with_tester(move |token| {
        script_payloads
            .get(token as usize)
            .map(|data| vec![ScriptSend::frame(PAUSE, FrameType::TestRequest, 0, 5, data)])
            .unwrap_or_default()
    });
    sim.run_until(ms(3_000));
    let frames = sim.frames();
    let requests: Vec<usize> = frames
        .iter()
        .enumerate()
        .filter(|(_, frame)| frame.is(FrameType::TestRequest, 5, 0))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(requests.len(), payloads.len(), "{}", log(frames));
    for (index, payload) in requests.into_iter().zip(&payloads) {
        let request = &frames[index];
        let response = &frames[index + 1];
        assert!(response.is(FrameType::TestResponse, 0, 5), "{response}");
        assert_eq!(&response.data, payload);
        assert_eq!(response.start, request.end + sim.timing().bits(40));
    }
    assert!(sim.deliveries(0).is_empty());
    sim.assert_no_violations();
}

/// Clause 9.5.6.2 (ReceivedUnwantedFrame): a Test_Request must not be
/// broadcast, and a master that receives one sends nothing back.
#[test]
fn a_broadcast_test_request_gets_no_answer() {
    let mut sim = with_tester(|token| {
        if token == 0 {
            vec![
                ScriptSend::frame(PAUSE, FrameType::TestRequest, 0xFF, 5, &[1, 2, 3]),
                ScriptSend::frame(millis(40), FrameType::Token, 0, 5, &[]),
            ]
        } else {
            Vec::new()
        }
    });
    sim.run_until(ms(2_000));
    let frames = sim.frames();
    assert!(frames
        .iter()
        .any(|frame| frame.is(FrameType::TestRequest, 5, 0xFF)));
    assert!(!frames
        .iter()
        .any(|frame| frame.frame_type() == Some(FrameType::TestResponse)));
    assert!(sim.deliveries(0).is_empty());
    sim.assert_no_violations();
}

/// Clause 9.5.6.2 (ReceivedDataNoReply) passes a Test_Response for this
/// station up to the higher layers. The core accepts one only as the
/// answer it is waiting for and otherwise drops it; #1617 tracks delivering
/// it. This documents the current behaviour.
#[test]
fn an_unsolicited_test_response_is_not_passed_up_today() {
    let mut sim = with_tester(|token| {
        if token == 0 {
            vec![
                ScriptSend::frame(PAUSE, FrameType::TestResponse, 0, 5, &[7, 7]),
                ScriptSend::frame(millis(40), FrameType::Token, 0, 5, &[]),
            ]
        } else {
            Vec::new()
        }
    });
    sim.run_until(ms(2_000));
    assert!(sim
        .frames()
        .iter()
        .any(|frame| frame.is(FrameType::TestResponse, 5, 0)));
    assert!(sim.deliveries(0).is_empty());
    sim.assert_no_violations();
}
