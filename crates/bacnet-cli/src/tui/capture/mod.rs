//! Live capture (#971): frames from the stack's own traffic and from the
//! MS/TP monitor, kept in a bounded ring for the Capture screen.
//!
//! - [`worker`]: the pumps that read a source, summarise each frame and
//!   hand them to the UI in batches, counting every frame lost on the way.
//! - [`ring`]: the bounded store of frames and their summaries, with the
//!   index of those the filter keeps.
//! - [`filter`]: the filter line's terms.
//!
//! Sources:
//!
//! - The frame tap of `bacnet-transport` (#957) on B/IP, B/IPv6, BACnet/SC
//!   and MS/TP: what this node sends and receives, and on MS/TP what it
//!   hears on the line. BACnet/SC frames are the plaintext, before TLS.
//! - The MS/TP monitor (#958, #970): every frame on the trunk, through its
//!   own lossy subscription beside the monitor screen's.
//!
//! Frames arrive summarised: the worker reads the headers, so the UI thread
//! only stores and draws. The Capture screen decodes the selected frame in
//! full, and only that one.

use std::time::SystemTime;

use bytes::Bytes;

pub(crate) use crate::decode::frame::{FrameSummary, Framing};

pub(crate) mod filter;
pub(crate) mod ring;
pub(crate) mod worker;

/// Frames the ring holds by default: past that the oldest go.
pub(crate) const DEFAULT_RING: usize = 50_000;

/// Which way a frame went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    /// Received by this node.
    In,
    /// Sent by this node.
    Out,
    /// Heard on the wire between other nodes (the monitor).
    Wire,
}

impl Direction {
    /// The list's short form.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
            Self::Wire => "--",
        }
    }
}

/// One captured frame, summarised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CaptureFrame {
    /// When the source saw it.
    pub(crate) time: SystemTime,
    /// The link, which decides how `bytes` are read.
    pub(crate) framing: Framing,
    /// Which way it went.
    pub(crate) direction: Direction,
    /// Where it came from, as the device table writes addresses.
    pub(crate) source: String,
    /// Where it went.
    pub(crate) destination: String,
    /// For a routed NPDU, whose ends `source` and `destination` give as
    /// routed stations (`2:0x05`): the link peer it went through.
    pub(crate) via: Option<String>,
    /// The frame as the link carries it; empty for a line error the
    /// monitor reports.
    pub(crate) bytes: Bytes,
    /// Its headers, read by the worker.
    pub(crate) summary: FrameSummary,
}

/// UI to worker: start capturing in this session. Idempotent; the capture
/// runs until the session ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureCommand {
    /// Subscribe to the session's source.
    Start,
}

/// Worker to UI.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CaptureEvent {
    /// A source is running.
    Started {
        /// What it captures.
        what: String,
        /// It belongs to the session, and ends with it.
        session: bool,
    },
    /// New frames, and frames lost since the last batch that got through.
    Frames(Box<Batch>),
    /// A source ended, and why.
    Ended(String),
}

/// Frames for the ring.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Batch {
    /// Oldest first.
    pub(crate) frames: Vec<CaptureFrame>,
    /// Frames lost before reaching the UI since the last batch it got.
    pub(crate) lost: u64,
}
