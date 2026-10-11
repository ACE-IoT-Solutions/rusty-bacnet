//! Matching and decoding of BVLC management responses for the B/IP client
//! helpers (`read_bdt`, `write_bdt`, `read_fdt`, `delete_fdt_entry`,
//! `register_foreign_device_bvlc`).

use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::bvll::BvllMessage;
use bacnet_bip_codec::{BvlcFunction, BvlcResultCode, Bvll};
use bacnet_types::error::Error;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BvlcResponseKind {
    Result,
    ReadBroadcastDistributionTableAck,
    ReadForeignDeviceTableAck,
}

impl BvlcResponseKind {
    pub(super) fn accepts(self, function: BvlcFunction) -> bool {
        match self {
            Self::Result => function == BvlcFunction::BVLC_RESULT,
            Self::ReadBroadcastDistributionTableAck => {
                function == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK
                    || function == BvlcFunction::BVLC_RESULT
            }
            Self::ReadForeignDeviceTableAck => {
                function == BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK
                    || function == BvlcFunction::BVLC_RESULT
            }
        }
    }
}

pub(super) struct PendingBvlcResponse {
    pub(super) owner: Arc<()>,
    pub(super) target: ([u8; 4], u16),
    pub(super) function: BvlcFunction,
    pub(super) expected: BvlcResponseKind,
    pub(super) ttl: Option<u16>,
    pub(super) deadline: Option<Instant>,
    /// At most one response received before local send completion; the bool
    /// records payload validity so accounting need not parse under its lock.
    pub(super) early_response: Option<(BvllMessage, bool)>,
    pub(super) tx: oneshot::Sender<BvllMessage>,
}

impl PendingBvlcResponse {
    pub(super) fn matches(&self, sender: ([u8; 4], u16), msg: &BvllMessage) -> bool {
        if self.target != sender || !self.expected.accepts(msg.function) {
            return false;
        }
        if msg.function != BvlcFunction::BVLC_RESULT {
            return true;
        }
        let Ok(code) = decode_bvlc_result_code(msg) else {
            return false;
        };
        // A Result answers this request only with the request's own NAK, or
        // with success when no ACK is expected (Annex J.2.1).
        let Some(nak) = self.function.nak() else {
            return false;
        };
        code == nak
            || (self.expected == BvlcResponseKind::Result
                && code == BvlcResultCode::SUCCESSFUL_COMPLETION)
    }
}

/// The error for a response of the wrong function.
pub(super) fn unexpected_response(expected: BvlcFunction, got: BvlcFunction) -> Error {
    Error::Encoding(format!("expected BVLC response {expected:?}, got {got:?}"))
}

/// The result code of a BVLC-Result, or why `msg` isn't a well-formed one.
pub(super) fn decode_bvlc_result_code(msg: &BvllMessage) -> Result<BvlcResultCode, Error> {
    if msg.function != BvlcFunction::BVLC_RESULT {
        return Err(unexpected_response(BvlcFunction::BVLC_RESULT, msg.function));
    }
    match msg.message()? {
        Bvll::BvlcResult(code) => Ok(code),
        other => Err(unexpected_response(
            BvlcFunction::BVLC_RESULT,
            other.function(),
        )),
    }
}
