//! The invariant checker: which frames the station may send, worked out
//! from the line and from the station's own frames, never from the core's
//! state or predicates.
//!
//! # Token custody
//!
//! The checker follows the token through the master state machine of
//! Clause 9.5.6 at the level of what the line shows, in three states:
//!
//! - **Free**: the station doesn't hold the token. It may answer what was
//!   asked of it (below), and may generate a token: a Poll For Master to
//!   TS + 1 once the line has been silent for T_no_token plus TS slots
//!   (GenerateToken, Clause 9.5.6.7), started only if no frame and at most
//!   Nmin_octets events came after it was handed over (SawFrame). Anything
//!   else is a send without the token.
//! - **Holding**: a Token for this station arrived, or the station's own
//!   poll made it the holder. It may send data frames, poll for masters and
//!   pass the token, each only once the reply its last frame asked for came
//!   or can no longer come: after a data request, a reply, an invalid frame
//!   or T_reply_timeout of silence (Clause 9.5.6.4); after a poll, an answer,
//!   an invalid frame or T_usage_timeout of silence (Clause 9.5.6.8). Any
//!   other frame heard while waiting drops the token (ReceivedUnexpectedFrame).
//! - **Passed**: its Token went out. It may retry it once, to the same
//!   station, and then poll for a new successor at NS + 1, each after
//!   T_usage_timeout of silence, and only while the successor shows no use of
//!   the token: no whole frame, and at most Nmin_octets events (Clause
//!   9.5.6.6; the core counts a retry's events from when it makes it).
//!
//! Where the line can't tell what the core did (a late deadline it didn't
//! service, a frame its decoder may or may not count as invalid, a frame of
//! the holder's it dropped unsent on its way back to IDLE), the checker
//! allows each choice: it may miss a fault there, but never reports a
//! correct core.
//!
//! # Limits
//!
//! - A token use carries at most Max_Info_Frames data frames (Clauses
//!   9.5.6.3, 9.5.6.5), and at most the core's local share of them,
//!   `multiple_frame_ratio`. A reply timeout ends the use (Clause 9.5.6.4).
//!   A sole master starts a use after each poll, Npoll uses between polls.
//! - Polls go to addresses up to Max_Master other than TS; consecutive polls
//!   go up one address at a time, starting again past TS when they reach it,
//!   so a sweep visits at most Max_Master addresses (Clauses 9.5.6.5,
//!   9.5.6.8). A new successor is sought at NS + 1, or TS + 1 when that is TS
//!   (Clause 9.5.6.6).
//! - Every frame starts T_turnaround after the last line activity (Clause
//!   9.5.5.1).
//!
//! # Answers
//!
//! Without the token the station may only answer: a Reply To Poll For Master
//! to a station that polled it, a Test_Response carrying a Test_Request's
//! data back to its sender, and Reply Postponed or the application's data
//! (Type 6, or 33 when it is longer than 501 octets) to a station whose data
//! request it received (Clauses 9.5.6.2, 9.5.6.9). Each answer goes once, and
//! its first octet ends before the asker's shortest timeout: T_reply_timeout
//! of 255 ms, or T_usage_timeout of 20 ms for a poll (Clause 9.5.3).
//!
//! # Deliveries
//!
//! Every delivery is a frame the checker heard for this station or
//! broadcast, in the order heard: data as an NPDU, a Test_Response or a
//! proprietary frame whole. Only a data request for this station carries a
//! request id.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use bacnet_mstp_codec::{FrameType, BROADCAST_MAC, MAX_STANDARD_MPDU_DATA};
use bacnet_mstp_core::RequestId;

use crate::line::{Heard, Line};
use crate::model::{Params, Seen, MIN_REPLY_TIMEOUT, N_MIN_OCTETS};
use crate::model::{T_NO_TOKEN, T_SLOT};
use crate::queues::QueueLog;

/// Who holds the token, as far as the line shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Custody {
    Free,
    Holding(Hold),
    Passed(Pass),
}

/// The station holds the token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hold {
    /// Data frames sent in this token use; for a possible sole master, since
    /// its last poll.
    frames: u64,
    /// What the last frame of this hold led to.
    after: After,
    /// The last frame of this hold was a poll, to this address.
    last_poll: Option<u8>,
    /// Polls in the current sweep.
    sweep: u64,
    /// A Reply To Poll For Master for this station came from this station's
    /// own address after its poll's wait ended. While a poll for the next
    /// address waits to go out, the core takes such a late answer as
    /// ReceivedReplyToPFM: from its own address, it then has no successor
    /// and starts a new sweep at TS + 1.
    own_reply: bool,
}

impl Hold {
    fn fresh() -> Self {
        Self {
            frames: 0,
            after: After::Fresh,
            last_poll: None,
            sweep: 0,
            own_reply: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    /// Nothing to wait for.
    Fresh,
    /// A frame of this hold is on the wire.
    Sending,
    /// A data request went out: WAIT_FOR_REPLY until a reply, an invalid
    /// frame or the timeout (`moved`). `data_ok`: the use may go on.
    Reply { moved: bool, data_ok: bool },
    /// A poll went out to `to`: POLL_FOR_MASTER until an answer, an invalid
    /// frame or the timeout (`moved`).
    Poll { to: u8, moved: bool },
    /// This station answered the poll: the token goes to it next.
    Successor(u8),
}

/// The token went out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pass {
    to: u8,
    retried: bool,
    /// Events count from here.
    base: u64,
}

/// What the station was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ask {
    Poll,
    Test,
    Reply,
}

#[derive(Debug, Clone)]
struct Owed {
    id: u64,
    ask: Ask,
    from: u8,
    /// When the asking frame ended.
    at: u64,
    /// The asking frame's data.
    data: Vec<u8>,
    /// The request id the delivery bore.
    request: Option<RequestId>,
    /// Data the application answered with.
    decided: Option<Vec<u8>>,
}

/// Why the station may send a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Answer(u64),
    Holder,
    Retry,
    FindSuccessor,
    Generate,
}

/// A frame handed to the adapter.
#[derive(Debug, Clone)]
struct Handover {
    octets: Vec<u8>,
    frame: Seen,
    role: Role,
    events: u64,
    frames: u64,
}

/// The checker.
pub struct Checker<'s> {
    /// The station's parameters.
    pub params: Params,
    /// The line as the station hears it.
    pub line: Line<'s>,
    queue: Rc<RefCell<QueueLog>>,
    custody: Custody,
    /// When `custody` last changed.
    since: u64,
    /// The core took back a frame of the holder's before its start, which
    /// it does on the way to IDLE.
    lapsed: bool,
    /// The station may be the sole master (DeclareSoleMaster).
    sole: bool,
    owed: Vec<Owed>,
    next_owed: u64,
    handover: Option<Handover>,
    deferred: Option<Handover>,
    in_flight: Option<Handover>,
    deliverable: VecDeque<Seen>,
    /// Where the run is, for failure messages.
    pub step: usize,
    /// The time, for failure messages.
    pub now: u64,
}

impl<'s> Checker<'s> {
    /// A checker for a station that has just been built.
    pub fn new(params: Params, line: Line<'s>, queue: Rc<RefCell<QueueLog>>) -> Self {
        Self {
            params,
            line,
            queue,
            custody: Custody::Free,
            since: 0,
            lapsed: false,
            sole: false,
            owed: Vec::new(),
            next_owed: 0,
            handover: None,
            deferred: None,
            in_flight: None,
            deliverable: VecDeque::new(),
            step: 0,
            now: 0,
        }
    }

    /// Report a broken invariant.
    pub fn fail(&self, what: &str) -> ! {
        panic!(
            "MS/TP core invariant broken at step {} ({} us): {what}\n  custody {:?}, sole {}",
            self.step, self.now, self.custody, self.sole
        );
    }

    /// Before each step: drop an expired partial frame, and note the waits
    /// that silence has ended.
    pub fn tick(&mut self, now: u64) {
        self.now = now;
        let before = self.custody;
        let mut heard = Vec::new();
        self.line.expire_partial(now, &mut heard);
        self.heard(heard, now);
        let silence = now.saturating_sub(self.line.last_activity);
        let (reply_timeout, usage_timeout) = (self.params.reply_timeout, self.params.usage_timeout);
        if let Custody::Holding(hold) = &mut self.custody {
            match &mut hold.after {
                After::Reply { moved, .. } if silence >= reply_timeout => *moved = true,
                After::Poll { to, moved } if silence >= usage_timeout => {
                    *moved = true;
                    let to = *to;
                    self.poll_ended(to);
                }
                _ => {}
            }
        }
        self.track(before, now);
    }

    /// Note when the custody changes.
    fn track(&mut self, before: Custody, now: u64) {
        if self.custody != before {
            self.since = now;
            self.lapsed = false;
        }
    }

    /// Whether the core may have dropped a frame of the holder's that waited
    /// to go out and gone back to IDLE, which the line needn't show: it
    /// drops one that can't start within T_no_token of being made (a retry
    /// or a new successor's poll within T_usage_timeout), and takes back a
    /// handed-over one on the way. The checker then allows what IDLE allows
    /// too.
    fn lapsible(&self, now: u64) -> bool {
        self.lapsed
            || match self.custody {
                Custody::Holding(hold) => {
                    !matches!(
                        hold.after,
                        After::Sending
                            | After::Reply { moved: false, .. }
                            | After::Poll { moved: false, .. }
                    ) && now >= self.since + T_NO_TOKEN
                }
                Custody::Passed(_) => now >= self.since + self.params.usage_timeout,
                Custody::Free => false,
            }
    }

    /// A poll to `to` went unanswered: after the last address before TS,
    /// the station may now be the sole master.
    fn poll_ended(&mut self, to: u8) {
        if self.params.next(to) == self.params.station {
            self.sole = true;
        }
    }

    /// What one input told the line.
    pub fn heard(&mut self, heard: Vec<Heard>, now: u64) {
        let before = self.custody;
        for event in heard {
            match event {
                Heard::Invalid => self.invalid_frame(),
                Heard::Frame(frame) => self.frame(frame, now),
            }
        }
        if let Custody::Passed(pass) = self.custody {
            // SawTokenUser: more than Nmin_octets events.
            if self.line.events - pass.base > N_MIN_OCTETS {
                self.custody = Custody::Free;
            }
        }
        self.track(before, now);
    }

    fn invalid_frame(&mut self) {
        let Custody::Holding(hold) = &mut self.custody else {
            return;
        };
        match &mut hold.after {
            After::Reply { moved, data_ok } => {
                *moved = true;
                *data_ok = true;
            }
            After::Poll { to, moved } => {
                *moved = true;
                let to = *to;
                self.poll_ended(to);
            }
            _ => {}
        }
    }

    fn frame(&mut self, frame: Seen, now: u64) {
        let station = self.params.station;
        if frame.destination == station {
            let ask = match frame.frame_type {
                FrameType::PollForMaster => Some(Ask::Poll),
                FrameType::TestRequest => Some(Ask::Test),
                _ if frame.expects_reply() => Some(Ask::Reply),
                _ => None,
            };
            if let Some(ask) = ask {
                // No answer may start once the asker's shortest timeout has
                // passed, so older questions are forgotten.
                self.owed.retain(|owed| owed.at + MIN_REPLY_TIMEOUT > now);
                self.owed.push(Owed {
                    id: self.next_owed,
                    ask,
                    from: frame.source,
                    at: now,
                    data: frame.data.clone(),
                    request: None,
                    decided: None,
                });
                self.next_owed += 1;
            }
        }
        if (frame.destination == station || frame.destination == BROADCAST_MAC)
            && (frame.carries_npdu()
                || frame.frame_type == FrameType::TestResponse
                || (frame.frame_type.is_proprietary() && frame.data.len() >= 2))
        {
            self.deliverable.push_back(frame.clone());
        }
        let token = frame.is(FrameType::Token, station);
        let reply = frame.destination == station
            && matches!(
                frame.frame_type,
                FrameType::BACnetDataNotExpectingReply
                    | FrameType::BACnetExtendedDataNotExpectingReply
                    | FrameType::TestResponse
                    | FrameType::ReplyPostponed
            );
        if matches!(self.custody, Custody::Passed(_)) {
            // SawTokenUser: a whole frame is more than Nmin_octets octets.
            self.custody = Custody::Free;
        }
        match &mut self.custody {
            Custody::Free | Custody::Passed(_) => {
                if token {
                    self.custody = Custody::Holding(Hold::fresh());
                }
            }
            Custody::Holding(hold) => match hold.after {
                After::Reply { moved: false, .. } if reply => {
                    hold.after = After::Reply {
                        moved: true,
                        data_ok: true,
                    };
                }
                After::Poll { moved: false, .. }
                    if frame.is(FrameType::ReplyToPollForMaster, station) =>
                {
                    hold.after = After::Successor(frame.source);
                    self.sole = false;
                }
                After::Reply { moved: false, .. } | After::Poll { moved: false, .. } => {
                    self.custody = Custody::Free;
                }
                After::Reply { moved: true, .. } if reply => {
                    hold.after = After::Reply {
                        moved: true,
                        data_ok: true,
                    };
                }
                After::Poll { moved: true, .. }
                    if frame.is(FrameType::ReplyToPollForMaster, station) =>
                {
                    hold.own_reply |= frame.source == station;
                }
                _ if token => *hold = Hold::fresh(),
                _ => {}
            },
        }
    }

    /// The core handed the adapter `octets`.
    pub fn handover(&mut self, octets: &[u8], now: u64) {
        self.now = now;
        let frame = Seen::decode(octets).unwrap_or_else(|error| self.fail(&error));
        if frame.source != self.params.station {
            self.fail(&format!("sent a frame from {}", frame.source));
        }
        // The frame a deferred start sent back to wait keeps the permission
        // it had, unless it is allowed afresh: the core may have dropped it
        // meanwhile and made an identical one since.
        let deferred = self
            .deferred
            .take()
            .filter(|deferred| deferred.octets == octets);
        let role = match (self.permit(&frame, now), deferred) {
            (Ok(role), _) => role,
            (Err(_), Some(deferred)) => {
                self.handover = Some(deferred);
                return;
            }
            (Err(why), None) => self.fail(&format!("{why} [{frame:?}]")),
        };
        if role == Role::Retry {
            let before = self.custody;
            if let Custody::Passed(pass) = &mut self.custody {
                pass.base = self.line.events;
            }
            self.track(before, now);
        }
        self.handover = Some(Handover {
            octets: octets.to_vec(),
            frame,
            role,
            events: self.line.events,
            frames: self.line.frames,
        });
    }

    fn permit(&self, frame: &Seen, now: u64) -> Result<Role, String> {
        match self.permit_held(frame, now) {
            Err(why)
                if frame.frame_type == FrameType::PollForMaster
                    && !matches!(self.custody, Custody::Free)
                    && self.lapsible(now) =>
            {
                self.generation(frame, now)
                    .map_err(|idle| format!("{why}; from IDLE, {idle}"))
            }
            other => other,
        }
    }

    fn permit_held(&self, frame: &Seen, now: u64) -> Result<Role, String> {
        match frame.frame_type {
            FrameType::ReplyToPollForMaster => self.answer(Ask::Poll, frame),
            FrameType::TestResponse => self.answer(Ask::Test, frame),
            FrameType::ReplyPostponed => self.answer(Ask::Reply, frame),
            FrameType::BACnetDataNotExpectingReply
            | FrameType::BACnetExtendedDataNotExpectingReply => {
                self.holder_data(frame).or_else(|holder| {
                    self.answer(Ask::Reply, frame)
                        .map_err(|answer| format!("{holder}; as an answer, {answer}"))
                })
            }
            FrameType::BACnetDataExpectingReply | FrameType::BACnetExtendedDataExpectingReply => {
                self.holder_data(frame)
            }
            FrameType::Token => self.token(frame, now),
            FrameType::PollForMaster => self.poll(frame, now),
            other => Err(format!(
                "sent a frame of type {other:?}, which this station never originates"
            )),
        }
    }

    fn answer(&self, ask: Ask, frame: &Seen) -> Result<Role, String> {
        if ask == Ask::Reply && frame.frame_type != FrameType::ReplyPostponed {
            let extended = frame.data.len() > MAX_STANDARD_MPDU_DATA;
            if frame.frame_type.is_cobs() != extended {
                return Err(format!(
                    "a reply of {} octets went as {:?}",
                    frame.data.len(),
                    frame.frame_type
                ));
            }
        }
        self.owed
            .iter()
            .rev()
            .find(|owed| {
                owed.ask == ask
                    && owed.from == frame.destination
                    && match ask {
                        Ask::Poll => true,
                        Ask::Test => owed.data == frame.data,
                        Ask::Reply => {
                            frame.frame_type == FrameType::ReplyPostponed
                                || owed.decided.as_deref() == Some(&frame.data[..])
                        }
                    }
            })
            .map(|owed| Role::Answer(owed.id))
            .ok_or_else(|| {
                format!(
                    "sent {:?} to {}, which asked for no such answer",
                    frame.frame_type, frame.destination
                )
            })
    }

    fn holder_data(&self, frame: &Seen) -> Result<Role, String> {
        let queued = self.queue.borrow().last_dequeued.clone();
        if queued.as_ref() != Some(&(frame.destination, frame.data.clone())) {
            return Err(format!(
                "a data frame that isn't the NPDU the queue handed out last ({queued:?})"
            ));
        }
        let extended = frame.data.len() > MAX_STANDARD_MPDU_DATA;
        if frame.frame_type.is_cobs() != extended {
            return Err(format!(
                "an NPDU of {} octets went as {:?}",
                frame.data.len(),
                frame.frame_type
            ));
        }
        if frame.destination != BROADCAST_MAC {
            // Clause 9.7.2: data_expecting_reply follows the NPDU's own bit.
            let expecting = frame.data.len() >= 2 && frame.data[1] & 0x04 != 0;
            if frame.expects_reply() != expecting {
                return Err("a data frame's type doesn't match its NPDU".into());
            }
        }
        self.may_send_data().map(|()| Role::Holder)
    }

    fn may_send_data(&self) -> Result<(), String> {
        let Custody::Holding(hold) = &self.custody else {
            return Err("sent a data frame without the token".into());
        };
        match hold.after {
            After::Fresh
            | After::Reply {
                moved: true,
                data_ok: true,
            } => Ok(()),
            After::Reply { moved: true, .. } | After::Poll { moved: true, .. } if self.sole => {
                Ok(())
            }
            After::Reply { moved: true, .. } => {
                Err("sent data after a reply timeout ended its token use".into())
            }
            After::Poll { moved: true, .. } => {
                Err("sent data after an unanswered poll, though not the sole master".into())
            }
            other => Err(format!("sent data while {other:?}")),
        }
    }

    fn token(&self, frame: &Seen, now: u64) -> Result<Role, String> {
        let to = frame.destination;
        if to == self.params.station || to == BROADCAST_MAC {
            return Err(format!("passed the token to {to}"));
        }
        match &self.custody {
            Custody::Holding(hold) => match hold.after {
                After::Fresh
                | After::Reply { moved: true, .. }
                | After::Poll { moved: true, .. } => Ok(Role::Holder),
                After::Successor(successor) if successor == to => Ok(Role::Holder),
                other => Err(format!("passed the token while {other:?}")),
            },
            Custody::Passed(pass) if pass.retried => Err("retried the token a second time".into()),
            Custody::Passed(pass) if pass.to != to => Err(format!(
                "retried the token to {to}, not to {}, its holder",
                pass.to
            )),
            Custody::Passed(_) => {
                let silence = now - self.line.last_activity;
                if silence < self.params.usage_timeout {
                    Err(format!(
                        "retried the token after {silence} us of silence, inside T_usage_timeout"
                    ))
                } else {
                    Ok(Role::Retry)
                }
            }
            Custody::Free => Err("passed a token it doesn't hold".into()),
        }
    }

    fn poll(&self, frame: &Seen, now: u64) -> Result<Role, String> {
        let station = self.params.station;
        let to = frame.destination;
        if to > self.params.max_master || to == station {
            return Err(format!(
                "polled {to} (Max_Master {})",
                self.params.max_master
            ));
        }
        let silence = now - self.line.last_activity;
        match &self.custody {
            Custody::Holding(hold) => match hold.after {
                After::Fresh
                | After::Reply { moved: true, .. }
                | After::Poll { moved: true, .. } => self.sweep(hold, to).map(|_| Role::Holder),
                After::Successor(successor) if successor == station => {
                    self.sweep(hold, to).map(|_| Role::Holder)
                }
                other => Err(format!("polled while {other:?}")),
            },
            Custody::Passed(pass) if !pass.retried => {
                Err("polled for a new successor before retrying the token".into())
            }
            Custody::Passed(pass) => {
                let next = self.params.next(pass.to);
                let expected = if next == station {
                    self.params.next(station)
                } else {
                    next
                };
                if silence < self.params.usage_timeout {
                    Err(format!(
                        "sought a new successor after {silence} us of silence, inside T_usage_timeout"
                    ))
                } else if to != expected {
                    Err(format!(
                        "sought a new successor for {} at {to}, not {expected}",
                        pass.to
                    ))
                } else {
                    Ok(Role::FindSuccessor)
                }
            }
            Custody::Free => self.generation(frame, now),
        }
    }

    /// GenerateToken (Clause 9.5.6.7).
    fn generation(&self, frame: &Seen, now: u64) -> Result<Role, String> {
        let station = self.params.station;
        let silence = now - self.line.last_activity;
        let needed = T_NO_TOKEN + T_SLOT * u64::from(station);
        if silence < needed {
            Err(format!(
                "generated a token after {silence} us of silence, short of T_no_token and {station} slots ({needed} us)"
            ))
        } else if frame.destination != self.params.next(station) {
            Err(format!(
                "generated a token with a poll to {}",
                frame.destination
            ))
        } else {
            Ok(Role::Generate)
        }
    }

    /// The sweep count after polling `to`.
    fn sweep(&self, hold: &Hold, to: u8) -> Result<u64, String> {
        let station = self.params.station;
        // A poll answered from this station's own address makes it its own
        // successor, which the core takes as a reason to poll from TS + 1.
        let restart = hold.after == After::Successor(station) || hold.own_reply;
        let count = match hold.last_poll {
            Some(previous) => {
                let next = self.params.next(previous);
                if next != station && to == next {
                    hold.sweep + 1
                } else if (next == station || restart) && to == self.params.next(station) {
                    1
                } else {
                    return Err(format!(
                        "polled {to} right after {previous}: a sweep goes up one address at a time"
                    ));
                }
            }
            None => 1,
        };
        if count > u64::from(self.params.max_master) {
            return Err(format!(
                "polled {count} addresses in one sweep, more than Max_Master ({})",
                self.params.max_master
            ));
        }
        Ok(count)
    }
}

mod sends;

pub use sends::Delivered;
