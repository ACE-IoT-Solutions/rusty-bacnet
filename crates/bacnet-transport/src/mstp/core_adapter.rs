use bacnet_mstp_codec::MAX_STANDARD_FRAME_LENGTH;
use bacnet_mstp_core::{AllocQueues, Instant as CoreInstant, MasterCore, ReplyDecision};
use bacnet_types::error::Error;

use super::{MasterNode, MasterState, MstpConfig};

fn core_error(error: impl core::fmt::Debug) -> Error {
    Error::Encoding(format!("MS/TP core error: {error:?}"))
}

pub(super) struct CoreAdapter {
    pub(super) core: MasterCore<'static, AllocQueues>,
    origin: tokio::time::Instant,
}

impl CoreAdapter {
    pub(super) fn new(config: &MstpConfig, origin: tokio::time::Instant) -> Result<Self, Error> {
        let config = config.to_core_config()?;
        let receive_storage = Box::leak(vec![0u8; MAX_STANDARD_FRAME_LENGTH].into_boxed_slice());
        let core = MasterCore::new(
            config,
            AllocQueues::new(256, 256),
            receive_storage,
            CoreInstant::ZERO,
        )
        .map_err(core_error)?;
        Ok(Self { core, origin })
    }

    pub(super) fn now(&self) -> CoreInstant {
        CoreInstant::from_micros(
            tokio::time::Instant::now()
                .duration_since(self.origin)
                .as_micros() as u64,
        )
    }

    pub(super) fn deadline(&self) -> Option<tokio::time::Instant> {
        let mut deadline = self.hard_deadline()?;
        let snapshot = self.core.snapshot();
        if snapshot.state == bacnet_mstp_core::MasterState::AnswerDataRequest
            && snapshot.request_id.is_some()
        {
            deadline = deadline
                .checked_sub(tokio::time::Duration::from_millis(
                    super::T_REPLY_TRANSMIT_MARGIN_MS,
                ))
                .unwrap_or(self.origin);
        }
        Some(deadline)
    }

    pub(super) fn hard_deadline(&self) -> Option<tokio::time::Instant> {
        self.core
            .next_deadline()
            .map(|deadline| self.origin + tokio::time::Duration::from_micros(deadline.as_micros()))
    }

    pub(super) fn service_deadline(&mut self) -> Result<(), bacnet_mstp_core::CoreError> {
        let now = self.now();
        let snapshot = self.core.snapshot();
        if snapshot.state == bacnet_mstp_core::MasterState::AnswerDataRequest {
            if let (Some(id), Some(hard)) = (snapshot.request_id, self.core.next_deadline()) {
                let margin =
                    bacnet_mstp_core::Duration::from_millis(super::T_REPLY_TRANSMIT_MARGIN_MS);
                if now < hard && hard.saturating_duration_since(now) <= margin {
                    return self
                        .core
                        .complete_request(id, ReplyDecision::Postponed, now);
                }
            }
        }
        self.core.on_deadline(now)
    }

    pub(super) fn sync_legacy(&self, legacy: &mut MasterNode) {
        let snapshot = self.core.snapshot();
        legacy.state = match snapshot.state {
            bacnet_mstp_core::MasterState::Idle => MasterState::Idle,
            bacnet_mstp_core::MasterState::NoToken => MasterState::NoToken,
            bacnet_mstp_core::MasterState::UseToken => MasterState::UseToken,
            bacnet_mstp_core::MasterState::DoneWithToken => MasterState::DoneWithToken,
            bacnet_mstp_core::MasterState::PassToken => MasterState::PassToken,
            bacnet_mstp_core::MasterState::WaitForReply => MasterState::WaitForReply,
            bacnet_mstp_core::MasterState::PollForMaster => MasterState::PollForMaster,
            bacnet_mstp_core::MasterState::AnswerDataRequest => MasterState::AnswerDataRequest,
        };
        legacy.next_station = snapshot.next_station;
        legacy.poll_station = snapshot.poll_station;
        legacy.frame_count = snapshot.frame_count;
        legacy.token_count = snapshot.token_count;
        legacy.sole_master = snapshot.sole_master;
        if snapshot.request_id.is_none() {
            legacy.pending_reply_source = None;
            legacy.reply_rx = None;
        }
    }
}
