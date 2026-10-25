//! Passwords kept out of the capture (#1799). The frame tap records what
//! this node sends, so a DeviceCommunicationControl or ReinitializeDevice
//! request from `:dcc` or `:reinit` would reach the ring, the detail and
//! hex views and a pcap export with its password in clear text.
//!
//! Before a frame this node did not receive itself enters the ring (one it
//! sent, or one libpcap or the monitor heard on the wire, which includes
//! this node's own sends), a confirmed request for either service has the
//! octets of its password overwritten with `*`. The length stays the same,
//! so every offset and length field still holds and the export still
//! parses; an MS/TP frame is encoded again so its data CRC matches. The
//! frame is marked, and the detail view says the password was redacted.
//!
//! A segmented request is left alone: neither request can be segmented,
//! since the largest (a DCC with a 20-character password) is about 35
//! octets of APDU, under the 50 every device accepts (Clause 20.1.2.5).
//! Frames this node received are other nodes' traffic and kept as heard.

use bacnet_encoding::tags::{TagClass, decode_tag};
use bacnet_services::device_mgmt::{DeviceCommunicationControlRequest, ReinitializeDeviceRequest};
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
    match link(framing, frame).npdu? {
        Npdu::At(range) if framing != Framing::Mstp => {
            let mut copy = frame.to_vec();
            mask_npdu(&mut copy[range]).then(|| Bytes::from(copy))
        }
        // An MS/TP frame's data has its own CRC, and an extended frame's
        // is COBS-encoded: mask the NPDU, then encode the frame again.
        Npdu::At(range) => mstp(frame, frame[range].to_vec()),
        Npdu::Encoded { data, .. } => mstp(frame, data),
    }
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
    mask_apdu(&mut npdu[offset..])
}

/// The context tag of each service's password, the request's last field.
fn password_tag(service: u8) -> Option<u8> {
    if service == ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL.to_raw() {
        Some(2)
    } else if service == ConfirmedServiceChoice::REINITIALIZE_DEVICE.to_raw() {
        Some(1)
    } else {
        None
    }
}

/// Mask the password of an unsegmented confirmed request for either
/// service: every octet of the string after its character set.
fn mask_apdu(apdu: &mut [u8]) -> bool {
    // An unsegmented Confirmed-Request: type 0, no segmentation bit.
    if apdu.len() < 4 || apdu[0] & 0xF8 != 0 {
        return false;
    }
    let Some(tag) = password_tag(apdu[3]) else {
        return false;
    };
    let body = &mut apdu[4..];
    let has_password = if tag == 2 {
        DeviceCommunicationControlRequest::decode(body).is_ok_and(|r| r.password.is_some())
    } else {
        ReinitializeDeviceRequest::decode(body).is_ok_and(|r| r.password.is_some())
    };
    if !has_password {
        return false;
    }
    let mut offset = 0;
    while let Ok((found, start)) = decode_tag(body, offset) {
        let end = start.saturating_add(found.length as usize).min(body.len());
        if found.class == TagClass::Context && found.number == tag && !found.is_opening {
            // The first content octet is the character set.
            let text = (start + 1).min(end)..end;
            body[text].fill(b'*');
            return true;
        }
        offset = end;
    }
    false
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::{EnableDisable, ReinitializedState};
    use bytes::BytesMut;

    use super::*;

    /// A B/IP unicast of `apdu`.
    fn bip(apdu: &[u8]) -> Vec<u8> {
        let npdu = [&[0x01, 0x04][..], apdu].concat();
        let length = (4 + npdu.len()) as u16;
        [&[0x81, 0x0A][..], &length.to_be_bytes(), &npdu].concat()
    }

    fn request(service: ConfirmedServiceChoice, body: &[u8]) -> Vec<u8> {
        [&[0x00, 0x05, 0x01, service.to_raw()][..], body].concat()
    }

    fn dcc(password: Option<&str>) -> Vec<u8> {
        let mut body = BytesMut::new();
        DeviceCommunicationControlRequest {
            time_duration: Some(30),
            enable_disable: EnableDisable::DISABLE_INITIATION,
            password: password.map(str::to_string),
        }
        .encode(&mut body)
        .unwrap();
        request(ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL, &body)
    }

    #[test]
    fn a_dcc_password_is_masked_in_place() {
        let frame = bip(&dcc(Some("Zq7-hunter2")));
        let masked = redacted(Framing::Bip, &frame).expect("masked");
        assert_eq!(masked.len(), frame.len());
        let text = String::from_utf8_lossy(&masked);
        assert!(!text.contains("hunter2"), "{masked:?}");
        let apdu = &masked[6..];
        let sent = DeviceCommunicationControlRequest::decode(&apdu[4..]).unwrap();
        assert_eq!(sent.password.as_deref(), Some("***********"));
        assert_eq!(sent.time_duration, Some(30));
        // Nothing to mask without a password, or in another service.
        assert_eq!(redacted(Framing::Bip, &bip(&dcc(None))), None);
        let read = request(ConfirmedServiceChoice::READ_PROPERTY, &[0x0C, 0, 0, 0, 1]);
        assert_eq!(redacted(Framing::Bip, &bip(&read)), None);
    }

    #[test]
    fn a_reinit_password_is_masked_and_an_mstp_frame_keeps_a_valid_crc() {
        let mut body = BytesMut::new();
        ReinitializeDeviceRequest {
            reinitialized_state: ReinitializedState::COLDSTART,
            password: Some("Zq7-hunter2-Xv9".into()),
        }
        .encode(&mut body)
        .unwrap();
        let npdu = [
            &[0x01, 0x04][..],
            &request(ConfirmedServiceChoice::REINITIALIZE_DEVICE, &body),
        ]
        .concat();
        let mut wire = vec![0; 64];
        let frame = Frame::new(FrameType::BACNET_DATA_EXPECTING_REPLY, 5, 1, &npdu);
        let length = frame.encode_into(&mut wire).unwrap();
        wire.truncate(length);
        let masked = redacted(Framing::Mstp, &wire).expect("masked");
        assert_eq!(masked.len(), wire.len());
        let (decoded, _) = bacnet_transport::mstp::codec::decode_frame(&masked).unwrap();
        let apdu = &decoded.data[2..];
        let sent = ReinitializeDeviceRequest::decode(&apdu[4..]).unwrap();
        assert_eq!(sent.password.as_deref(), Some("***************"));
        assert_eq!(sent.reinitialized_state, ReinitializedState::COLDSTART);
    }

    /// Frames this node received are kept as heard; its own are scrubbed.
    #[test]
    fn only_frames_not_received_here_are_scrubbed() {
        let frame = |direction| CaptureFrame {
            time: std::time::UNIX_EPOCH,
            framing: Framing::Bip,
            direction,
            udp: None,
            source: String::new(),
            destination: String::new(),
            via: None,
            bytes: Bytes::from(bip(&dcc(Some("Zq7-hunter2")))),
            summary: Default::default(),
            redacted: false,
        };
        assert!(!scrub(frame(Direction::In)).redacted);
        for direction in [Direction::Out, Direction::Wire] {
            let scrubbed = scrub(frame(direction));
            assert!(scrubbed.redacted);
            assert!(!String::from_utf8_lossy(&scrubbed.bytes).contains("hunter2"));
        }
    }
}
