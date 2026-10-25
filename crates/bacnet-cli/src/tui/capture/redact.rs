//! Passwords kept out of the capture (#1799). The frame tap records what
//! this node sends, so a DeviceCommunicationControl or ReinitializeDevice
//! request from `:dcc` or `:reinit` would reach the ring, the detail and
//! hex views and a pcap export with its password in clear text.
//!
//! Before a frame this node did not receive itself enters the ring (one it
//! sent, or one libpcap or the monitor heard on the wire, which includes
//! this node's own sends), a confirmed request for either service has its
//! password's content octets overwritten with `*`, whatever the character
//! set. The masking fails closed (#2062):
//!
//! - The request's tags are walked over their raw length octets, not
//!   decoded, so a password too long, cut short, non-canonically encoded or
//!   in an encoding the decoder refuses is masked too. Only the fields that
//!   come before the password, at their expected sizes, are walked past:
//!   anything else ends the walk and is masked to the end of the APDU.
//! - A segmented request is masked too (#2084). The password is each
//!   service's last field, so it starts in the first segment and anything
//!   of it that a later segment carries is that segment's whole service
//!   data: the first segment is masked from the password on, every later
//!   one entirely. No transaction is tracked.
//! - A frame whose link header doesn't decode (a B/IP length that disagrees
//!   with the datagram, an MS/TP frame with a bad CRC) is searched for
//!   anything shaped like either request, at every offset, and each one
//!   found is masked.
//!
//! B/IP, B/IPv6 and BACnet/SC frames keep their length, so the export still
//! parses. A valid MS/TP frame is encoded again so its data CRC matches; an
//! extended frame's COBS encoding may then differ in length. Should that
//! encoding fail, the frame is masked in place instead. A damaged MS/TP
//! frame is masked in place and keeps its wrong CRC: a standard frame's
//! data as it stands, an extended frame's by decoding its COBS data without
//! the checks and writing each masked octet back where it came from. A zero
//! that a COBS code octet stands for has no octet of its own and stays.
//! The frame is marked, and the detail view says the password was redacted.
//! Frames this node received are other nodes' traffic and kept as heard.

use std::ops::Range;

use bacnet_transport::mstp::codec::crc::crc8_valid;
use bacnet_transport::mstp::codec::{Frame, FrameType, PREAMBLE, decode_frame_in_place};
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
    if framing == Framing::Mstp {
        return mstp(frame, encode);
    }
    if !might_carry(frame) {
        return None;
    }
    let link = link(framing, frame);
    let mut copy = frame.to_vec();
    let masked = match link.npdu {
        Some(Npdu::At(range)) => mask_npdu(&mut copy[range]),
        // A link message without an NPDU carries no request; one whose
        // header didn't decode might, anywhere.
        _ => link.malformed && scan(&mut copy),
    };
    masked.then(|| Bytes::from(copy))
}

/// A cheap look before parsing: octets shaped like a confirmed request for
/// either service, at any offset.
fn might_carry(octets: &[u8]) -> bool {
    (0..octets.len()).any(|at| request(&octets[at..]).is_some())
}

/// A confirmed request for either service, as its first octets show it.
struct Request {
    /// The password's context tag.
    password: u8,
    /// The context tags that may come before it, each with its largest
    /// content length.
    before: &'static [(u8, usize)],
    /// Where the service parameters start.
    body: usize,
    /// A segment after the first.
    later_segment: bool,
}

/// `apdu` read as a confirmed request for either service, segmented or not.
fn request(apdu: &[u8]) -> Option<Request> {
    let first = *apdu.first()?;
    if first & 0xF0 != 0 {
        return None;
    }
    // A segment adds a sequence number and a window size before the service.
    let segmented = first & 0x08 != 0;
    let (service, body) = if segmented { (5, 6) } else { (3, 4) };
    let service = ConfirmedServiceChoice::from_raw(*apdu.get(service)?);
    let (password, before): (u8, &[(u8, usize)]) =
        if service == ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL {
            // Time duration (an Unsigned16, leniently up to 4), then
            // enable/disable.
            (2, &[(0, 4), (1, 1)])
        } else if service == ConfirmedServiceChoice::REINITIALIZE_DEVICE {
            // The state to reinitialize to.
            (1, &[(0, 1)])
        } else {
            return None;
        };
    Some(Request {
        password,
        before,
        body,
        later_segment: segmented && apdu[3] != 0,
    })
}

/// Mask the password of the DCC or ReinitializeDevice request in `npdu`;
/// true when anything was. An NPDU that doesn't decode is searched.
fn mask_npdu(npdu: &mut [u8]) -> bool {
    match bacnet_npdu::Npdu::decode(npdu) {
        Ok(view) if view.message_type.is_some() => false,
        Ok(view) => {
            let offset = view.payload_offset;
            npdu.get_mut(offset..).is_some_and(mask_apdu)
        }
        Err(_) => scan(npdu),
    }
}

/// Mask every request for either service found at any offset of `octets`;
/// true when anything was. A `*` never starts a request or names either
/// service, so masking one can't make another appear.
fn scan(octets: &mut [u8]) -> bool {
    let mut masked = false;
    for at in 0..octets.len() {
        masked |= mask_apdu(&mut octets[at..]);
    }
    masked
}

/// Mask the password of a confirmed request for either service: every
/// content octet of its tag after the character set, as far as the APDU
/// holds them; from the first tag that isn't an expected field to the end;
/// and all of a later segment. True when anything was masked. The walk
/// always moves forward (a tag takes at least its first octet), and every
/// end is clamped.
fn mask_apdu(apdu: &mut [u8]) -> bool {
    let Some(request) = request(apdu) else {
        return false;
    };
    let body = &mut apdu[request.body..];
    if request.later_segment {
        return fill(body, 0);
    }
    let mut at = 0;
    while at < body.len() {
        let Some(tag) = raw_tag(body, at) else {
            return fill(body, at);
        };
        let end = tag.content.saturating_add(tag.length).min(body.len());
        if tag.context && !tag.bracket {
            if tag.number == request.password {
                // The first content octet is the character set.
                let text = (tag.content + 1).min(end)..end;
                body[text.clone()].fill(b'*');
                return !text.is_empty();
            }
            let expected = request
                .before
                .iter()
                .any(|&(number, most)| number == tag.number && tag.length <= most);
            if expected {
                at = end;
                continue;
            }
        }
        return fill(body, tag.content);
    }
    false
}

/// Mask `octets` from `start` to the end; true when that wasn't empty.
fn fill(octets: &mut [u8], start: usize) -> bool {
    let start = start.min(octets.len());
    let rest = &mut octets[start..];
    rest.fill(b'*');
    !rest.is_empty()
}

/// A tag as its header octets say, unchecked.
struct RawTag {
    number: u8,
    context: bool,
    /// An opening or closing tag.
    bracket: bool,
    /// Where its content starts, at most the end of the octets.
    content: usize,
    /// The content length its header claims.
    length: usize,
}

/// The tag whose header starts at `at`, read from the raw length octets
/// with none of the decoder's checks; `None` when the header runs past the
/// end. An application tag's length isn't read: it is never walked past.
fn raw_tag(octets: &[u8], at: usize) -> Option<RawTag> {
    let first = *octets.get(at)?;
    let mut next = at + 1;
    let mut number = first >> 4;
    if number == 0x0F {
        number = *octets.get(next)?;
        next += 1;
    }
    let context = first & 0x08 != 0;
    let bracket = context && (first & 0x07) >= 6;
    let length = match first & 0x07 {
        _ if !context || bracket => 0,
        short @ 0..=4 => usize::from(short),
        _ => {
            let extended = *octets.get(next)?;
            next += 1;
            let width = match extended {
                254 => 2,
                255 => 4,
                short => {
                    return Some(RawTag {
                        number,
                        context,
                        bracket,
                        content: next,
                        length: usize::from(short),
                    });
                }
            };
            let wide = octets.get(next..next + width)?;
            next += width;
            wide.iter()
                .fold(0usize, |sum, &octet| (sum << 8) | usize::from(octet))
        }
    };
    Some(RawTag {
        number,
        context,
        bracket,
        content: next,
        length,
    })
}

/// How an MS/TP frame is encoded again around its masked data.
type Encoder = fn(Frame<'_>) -> Option<Bytes>;

/// `frame` encoded, so its data CRC matches its masked data.
fn encode(frame: Frame<'_>) -> Option<Bytes> {
    let mut wire = vec![0; frame.data.len() + frame.data.len() / 128 + 16];
    let length = frame.encode_into(&mut wire).ok()?;
    wire.truncate(length);
    Some(Bytes::from(wire))
}

/// The two data frame types and their extended forms: those that carry an NPDU.
fn carries_npdu(frame_type: FrameType) -> bool {
    matches!(
        frame_type,
        FrameType::BACNET_DATA_EXPECTING_REPLY
            | FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
            | FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY
            | FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
    )
}

/// The type and Length of an MS/TP frame whose preamble and header CRC are
/// sound.
fn sound_header(frame: &[u8]) -> Option<(FrameType, usize)> {
    let header = frame.get(..8)?;
    (header[..2] == PREAMBLE && crc8_valid(&header[2..8])).then(|| {
        (
            FrameType::from_raw(header[2]),
            usize::from(u16::from_be_bytes([header[5], header[6]])),
        )
    })
}

/// `start..end`, clamped to `length` and never backwards.
fn span(start: usize, end: usize, length: usize) -> Range<usize> {
    let end = end.min(length);
    start.min(end)..end
}

/// Where an extended frame with this Length holds its Encoded Data field:
/// the Length counts it and the five octets of the encoded CRC, less two.
fn encoded_data(frame: &[u8], length: usize) -> Range<usize> {
    span(8, length + 5, frame.len())
}

/// An MS/TP frame with its password masked. A frame with a sound header
/// that names a standard frame without such a request is left alone at
/// once; an extended frame's data is COBS-encoded, so it is always read.
fn mstp(frame: &[u8], encode: Encoder) -> Option<Bytes> {
    let header = sound_header(frame);
    if let Some((frame_type, _)) = header
        && !frame_type.is_cobs()
        && !(carries_npdu(frame_type) && might_carry(frame))
    {
        return None;
    }
    let mut decoded = frame.to_vec();
    match (decode_frame_in_place(&mut decoded), header) {
        (Ok((sound, _)), Some((_, length))) => {
            if !carries_npdu(sound.header.frame_type) {
                return None;
            }
            let mut data = sound.data.to_vec();
            if !mask_npdu(&mut data) {
                return None;
            }
            encode(Frame {
                header: sound.header,
                data: &data,
            })
            .or_else(|| Some(mstp_in_place(frame, sound.header.frame_type, length, &data)))
        }
        _ => damaged_mstp(frame, header),
    }
}

/// A valid MS/TP frame with its masked `data` written over its own, when
/// encoding it again failed: the data CRC is then wrong, but nothing of
/// the password is left.
fn mstp_in_place(frame: &[u8], frame_type: FrameType, length: usize, data: &[u8]) -> Bytes {
    let mut copy = frame.to_vec();
    if frame_type.is_cobs() {
        let field = encoded_data(frame, length);
        put_back(&mut copy[field.clone()], &cobs_decode(&frame[field]), data);
    } else {
        copy[8..8 + data.len()].copy_from_slice(data);
    }
    Bytes::from(copy)
}

/// An MS/TP frame that didn't decode, masked in place. With a sound header
/// its data is where the header says, plain or COBS-encoded by its type,
/// and a frame without data is left alone. Without one, both readings are
/// searched: the octets as they stand, and decoded as COBS from the end of
/// the header.
fn damaged_mstp(frame: &[u8], header: Option<(FrameType, usize)>) -> Option<Bytes> {
    let (plain, encoded) = match header {
        Some((frame_type, length)) if frame_type.is_cobs() => {
            (None, Some(encoded_data(frame, length)))
        }
        Some((frame_type, length)) if carries_npdu(frame_type) => {
            (Some(span(8, 8 + length, frame.len())), None)
        }
        Some(_) => return None,
        None => (
            Some(0..frame.len()),
            Some(span(8, frame.len().saturating_sub(5), frame.len())),
        ),
    };
    let mut copy = frame.to_vec();
    let mut masked = plain.is_some_and(|range| scan(&mut copy[range]));
    if let Some(field) = encoded {
        let decoded = cobs_decode(&frame[field.clone()]);
        let mut data: Vec<u8> = decoded.iter().map(|&(octet, _)| octet).collect();
        if scan(&mut data) {
            put_back(&mut copy[field], &decoded, &data);
            masked = true;
        }
    }
    masked.then(|| Bytes::from(copy))
}

/// What each octet of an encoded field is XORed with on the wire.
const COBS_MASK: u8 = 0x55;

/// The octets a COBS-encoded field decodes to, each with its offset in the
/// field, or `None` for a zero that a code octet stands for. Nothing is
/// checked: a zero code is read as one, and a block the field cuts short
/// keeps what it holds.
fn cobs_decode(field: &[u8]) -> Vec<(u8, Option<usize>)> {
    let mut decoded = Vec::with_capacity(field.len());
    let mut at = 0;
    while at < field.len() {
        let code = usize::from(field[at] ^ COBS_MASK).max(1);
        let end = (at + code).min(field.len());
        decoded.extend((at + 1..end).map(|i| (field[i] ^ COBS_MASK, Some(i))));
        at = end;
        if code != 0xFF && at < field.len() {
            decoded.push((0, None));
        }
    }
    decoded
}

/// Write each octet of `data` that differs from `decoded` back into the
/// encoded `field` where it came from.
fn put_back(field: &mut [u8], decoded: &[(u8, Option<usize>)], data: &[u8]) {
    for (&(old, at), &new) in decoded.iter().zip(data) {
        if let Some(at) = at
            && old != new
        {
            field[at] = new ^ COBS_MASK;
        }
    }
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "redact_fail_closed_tests.rs"]
mod fail_closed_tests;
