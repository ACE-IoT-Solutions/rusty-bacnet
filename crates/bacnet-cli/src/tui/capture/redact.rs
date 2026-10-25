//! Passwords kept out of the capture (#1799). The frame tap records what
//! this node sends, so a DeviceCommunicationControl or ReinitializeDevice
//! request from `:dcc` or `:reinit` would reach the ring, the detail and
//! hex views and a pcap export with its password in clear text.
//!
//! Before a frame this node did not receive itself enters the ring (one it
//! sent, or one libpcap or the monitor heard on the wire, which includes
//! this node's own sends), a confirmed request for either service has the
//! content octets of its password tag overwritten with `*`, whatever the
//! character set, and however the rest of the request is formed: the tags
//! are walked to the password's, nothing is decoded first, so a password
//! too long, cut short, or in an encoding the decoder refuses is masked
//! too. B/IP, B/IPv6 and BACnet/SC frames keep their length, so the export
//! still parses. A valid MS/TP frame is encoded again so its data CRC
//! matches; an extended frame's COBS encoding may then differ in length.
//! An MS/TP frame whose data CRC was already wrong is masked in place by
//! its header's length and keeps its wrong CRC; an extended frame that
//! doesn't decode is left alone, since its COBS-encoded data can't be read
//! in place. The frame is marked, and
//! the detail view says the password was redacted.
//!
//! A segmented request is left alone: neither request can be segmented,
//! since the largest (a DCC with a 20-character password) is about 35
//! octets of APDU, under the 50 every device accepts (Clause 20.1.2.5).
//! Frames this node received are other nodes' traffic and kept as heard.

use bacnet_encoding::tags::{TagClass, decode_tag};
use bacnet_transport::mstp::codec::crc::crc8_valid;
use bacnet_transport::mstp::codec::{Frame, FrameType};
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::Bytes;

use super::{CaptureFrame, Direction};
use crate::decode::frame::{Framing, Npdu, link};

/// Redact `frame`'s password, if it carries one and wasn't received here.
pub(crate) fn scrub(mut frame: CaptureFrame) -> CaptureFrame {
    if frame.direction != Direction::In
        && let Some(bytes) = redacted(frame.framing, &frame.bytes)
    {
        frame.bytes = bytes;
        frame.redacted = true;
    }
    frame
}

/// The frame with its password masked, or `None` when it carries none.
pub(crate) fn redacted(framing: Framing, frame: &[u8]) -> Option<Bytes> {
    if !might_carry(framing, frame) {
        return None;
    }
    match link(framing, frame).npdu {
        Some(Npdu::At(range)) if framing != Framing::Mstp => {
            let mut copy = frame.to_vec();
            mask_npdu(&mut copy[range]).then(|| Bytes::from(copy))
        }
        // An MS/TP frame's data has its own CRC, and an extended frame's
        // is COBS-encoded: mask the NPDU, then encode the frame again.
        Some(Npdu::At(range)) => mstp(frame, frame[range].to_vec()),
        Some(Npdu::Encoded { data, .. }) => mstp(frame, data),
        None if framing == Framing::Mstp => mstp_in_place(frame),
        None => None,
    }
}

/// The two services' numbers.
const SERVICES: [u8; 2] = [
    ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL.to_raw(),
    ConfirmedServiceChoice::REINITIALIZE_DEVICE.to_raw(),
];

/// A cheap look before parsing: four octets shaped like an unsegmented
/// confirmed request for either service. An extended MS/TP frame's data is
/// COBS-encoded, so it is always parsed.
fn might_carry(framing: Framing, frame: &[u8]) -> bool {
    if framing == Framing::Mstp
        && frame
            .get(2)
            .is_some_and(|t| FrameType::from_raw(*t).is_cobs())
    {
        return true;
    }
    frame
        .windows(4)
        .any(|w| w[0] & 0xF8 == 0 && SERVICES.contains(&w[3]))
}

/// `frame` encoded again around `npdu`, masked; `None` when nothing was.
fn mstp(frame: &[u8], mut npdu: Vec<u8>) -> Option<Bytes> {
    if !mask_npdu(&mut npdu) {
        return None;
    }
    let frame_type = FrameType::from_raw(*frame.get(2)?);
    let encoded = Frame::new(frame_type, *frame.get(3)?, *frame.get(4)?, &npdu);
    let mut wire = vec![0; frame.len() + 16 + npdu.len() / 128];
    let length = encoded.encode_into(&mut wire).ok()?;
    wire.truncate(length);
    Some(Bytes::from(wire))
}

/// An MS/TP frame that didn't decode, most likely for its data CRC: when
/// its header is sound and names a data frame, mask the data in place by
/// the header's length, as far as the frame holds it.
fn mstp_in_place(frame: &[u8]) -> Option<Bytes> {
    let header = frame.get(..8)?;
    if header[..2] != [0x55, 0xFF] || !crc8_valid(&header[2..8]) {
        return None;
    }
    let data = matches!(
        FrameType::from_raw(header[2]),
        FrameType::BACNET_DATA_EXPECTING_REPLY | FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
    );
    let length = usize::from(u16::from_be_bytes([header[5], header[6]]));
    if !data || length == 0 {
        return None;
    }
    let mut copy = frame.to_vec();
    let end = (8 + length).min(copy.len());
    mask_npdu(&mut copy[8..end]).then(|| Bytes::from(copy))
}

/// Mask the password of the DCC or ReinitializeDevice request in `npdu`;
/// true when there was one.
fn mask_npdu(npdu: &mut [u8]) -> bool {
    let Ok(view) = bacnet_npdu::Npdu::decode(npdu) else {
        return false;
    };
    if view.message_type.is_some() {
        return false;
    }
    let offset = view.payload_offset;
    npdu.get_mut(offset..).is_some_and(mask_apdu)
}

/// The context tag of each service's password, the request's last field.
fn password_tag(service: u8) -> Option<u8> {
    if service == SERVICES[0] {
        Some(2)
    } else if service == SERVICES[1] {
        Some(1)
    } else {
        None
    }
}

/// Mask the password of an unsegmented confirmed request for either
/// service: every content octet of its tag after the character set, as
/// far as the APDU holds them. The walk always moves forward (a tag
/// consumes at least its first octet), and every end is clamped.
fn mask_apdu(apdu: &mut [u8]) -> bool {
    // An unsegmented Confirmed-Request: type 0, no segmentation bit.
    if apdu.len() < 4 || apdu[0] & 0xF8 != 0 {
        return false;
    }
    let Some(tag) = password_tag(apdu[3]) else {
        return false;
    };
    let body = &mut apdu[4..];
    let mut offset = 0;
    while let Ok((found, start)) = decode_tag(body, offset) {
        let end = start.saturating_add(found.length as usize).min(body.len());
        let password = found.class == TagClass::Context
            && found.number == tag
            && !found.is_opening
            && !found.is_closing;
        if password {
            // The first content octet is the character set.
            let text = (start + 1).min(end)..end;
            if text.is_empty() {
                return false;
            }
            body[text].fill(b'*');
            return true;
        }
        offset = end;
    }
    false
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;
