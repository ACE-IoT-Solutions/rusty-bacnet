//! Helpers that read token behaviour off the wire log.

use std::collections::BTreeSet;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::Instant;

use crate::line::Driver;
use crate::report::WireFrame;

/// One stretch in which a station held the token, read off the wire: from a
/// clean Token addressed to it (or its own token-generation poll) to the
/// Token it sent on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    /// When the token arrived (the end of the Token frame), or when the
    /// station's generation poll started.
    pub start: Instant,
    /// Data and test frames the station sent while holding the token.
    pub info_frames: usize,
    /// Polls For Master it sent while holding it.
    pub polls: usize,
    /// Where it passed the token, once it did.
    pub passed_to: Option<u8>,
}

/// Every clean Token frame: when it started, from where, to where.
#[must_use]
pub fn token_passes(frames: &[WireFrame]) -> Vec<(Instant, u8, u8)> {
    frames
        .iter()
        .filter(|frame| frame.is_clean() && frame.frame_type() == Some(FrameType::Token))
        .filter_map(|frame| Some((frame.start, frame.source()?, frame.destination()?)))
        .collect()
}

/// Stations that used a token at or after `since`: a clean Token was
/// addressed to them and the next frame on the wire was theirs. A Token sent
/// to a dead or deaf station does not count.
#[must_use]
pub fn token_users(frames: &[WireFrame], since: Instant) -> BTreeSet<u8> {
    frames
        .windows(2)
        .filter(|pair| pair[0].start >= since && pair[0].is_clean())
        .filter(|pair| pair[0].frame_type() == Some(FrameType::Token))
        .filter_map(|pair| {
            let to = pair[0].destination()?;
            (pair[1].driver == Driver::Station(to)).then_some(to)
        })
        .collect()
}

/// The token holds of `station`, in order. A hold that has not ended has no
/// `passed_to`.
#[must_use]
pub fn holds(frames: &[WireFrame], station: u8) -> Vec<Hold> {
    let mut holds = Vec::new();
    let mut current: Option<Hold> = None;
    for frame in frames.iter().filter(|frame| frame.is_clean()) {
        let Some(header) = frame.header else {
            continue;
        };
        let ours = frame.driver == Driver::Station(station);
        if header.frame_type == FrameType::Token && header.destination == station {
            if let Some(open) = current.take() {
                holds.push(open);
            }
            current = Some(Hold {
                start: frame.end,
                info_frames: 0,
                polls: 0,
                passed_to: None,
            });
            continue;
        }
        if ours && frame.generation {
            if let Some(open) = current.take() {
                holds.push(open);
            }
            current = Some(Hold {
                start: frame.start,
                info_frames: 0,
                polls: 1,
                passed_to: None,
            });
            continue;
        }
        let Some(hold) = current.as_mut().filter(|_| ours) else {
            continue;
        };
        match header.frame_type {
            FrameType::Token => {
                hold.passed_to = Some(header.destination);
                holds.extend(current.take());
            }
            FrameType::PollForMaster => hold.polls += 1,
            FrameType::BACnetDataExpectingReply
            | FrameType::BACnetDataNotExpectingReply
            | FrameType::TestRequest
            | FrameType::Unknown(_) => hold.info_frames += 1,
            FrameType::ReplyToPollForMaster
            | FrameType::TestResponse
            | FrameType::ReplyPostponed => {}
        }
    }
    holds.extend(current);
    holds
}

/// Lengths of the runs of consecutive holds without a Poll For Master that
/// lie between two holds with one: the token uses between maintenance
/// polls.
#[must_use]
pub fn holds_between_polls(holds: &[Hold]) -> Vec<usize> {
    let polled: Vec<usize> = holds
        .iter()
        .enumerate()
        .filter(|(_, hold)| hold.polls > 0)
        .map(|(index, _)| index)
        .collect();
    polled
        .windows(2)
        .map(|pair| pair[1] - pair[0] - 1)
        .filter(|&gap| gap > 0)
        .collect()
}
