//! `BACnetEventParameter` full ASN.1 framing.
//!
//! `BACnetEventParameter` (ASHRAE 135-2020 Clause 21) selects one event
//! algorithm by context tag and carries that algorithm's settings. The tag
//! is also the number of the event type the algorithm reports. Every
//! alternative but `none` is a constructed value under its own tag, holding
//! context-tagged members that are all required:
//!
//! | Tag | Alternative | Member tag | Member | Type |
//! |---|---|---|---|---|
//! | `[0]` | `change-of-bitstring` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]` | `bitmask` | bit string |
//! | | | `[2]` | `list-of-bitstring-values` | list of bit strings |
//! | `[1]` | `change-of-state` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]` | `list-of-values` | list of `BACnetPropertyStates` |
//! | `[2]` | `change-of-value` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]` | `cov-criteria` | `[0]` bit string or `[1]` REAL, see below |
//! | `[3]` | `command-failure` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]` | `feedback-property-reference` | `BACnetDeviceObjectPropertyReference` |
//! | `[4]` | `floating-limit` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]` | `setpoint-reference` | `BACnetDeviceObjectPropertyReference` |
//! | | | `[2]` | `low-diff-limit` | REAL |
//! | | | `[3]` | `high-diff-limit` | REAL |
//! | | | `[4]` | `deadband` | REAL |
//! | `[5]` | `out-of-range` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]` | `low-limit` | REAL |
//! | | | `[2]` | `high-limit` | REAL |
//! | | | `[3]` | `deadband` | REAL |
//! | `[8]` | `change-of-life-safety` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]` | `list-of-life-safety-alarm-values` | list of `BACnetLifeSafetyState` |
//! | | | `[2]` | `list-of-alarm-values` | list of `BACnetLifeSafetyState` |
//! | | | `[3]` | `mode-property-reference` | `BACnetDeviceObjectPropertyReference` |
//! | `[9]` | `extended` | `[0]` | `vendor-id` | Unsigned16 |
//! | | | `[1]` | `extended-event-type` | Unsigned |
//! | | | `[2]` | `parameters` | list of extended values, see below |
//! | `[10]` | `buffer-ready` | `[0]` | `notification-threshold` | Unsigned |
//! | | | `[1]` | `previous-notification-count` | Unsigned32 |
//! | `[11]` | `unsigned-range` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]` | `low-limit` | Unsigned |
//! | | | `[2]` | `high-limit` | Unsigned |
//! | `[13]` | `access-event` | `[0]` | `list-of-access-events` | list of `BACnetAccessEvent` |
//! | | | `[1]` | `access-event-time-reference` | `BACnetDeviceObjectPropertyReference` |
//! | `[14]` | `double-out-of-range` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]`, `[2]`, `[3]` | `low-limit`, `high-limit`, `deadband` | Double |
//! | `[15]` | `signed-out-of-range` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]`, `[2]` | `low-limit`, `high-limit` | INTEGER |
//! | | | `[3]` | `deadband` | Unsigned |
//! | `[16]` | `unsigned-out-of-range` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]`, `[2]`, `[3]` | `low-limit`, `high-limit`, `deadband` | Unsigned |
//! | `[17]` | `change-of-characterstring` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]` | `list-of-alarm-values` | list of CharacterString |
//! | `[18]` | `change-of-status-flags` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]` | `selected-flags` | `BACnetStatusFlags` |
//! | `[20]` | `none` | | | a primitive NULL, no members |
//! | `[21]` | `change-of-discrete-value` | `[0]` | `time-delay` | Unsigned |
//! | `[22]` | `change-of-timer` | `[0]` | `time-delay` | Unsigned |
//! | | | `[1]` | `alarm-values` | list of `BACnetTimerState` |
//! | | | `[2]` | `update-time-reference` | `BACnetDeviceObjectPropertyReference` |
//!
//! A list is its member's opening and closing tag around application-tagged
//! items, and a reference its member's tags around the reference's own
//! members. `cov-criteria` is either `[0]` `bitmask` (a bit string) or `[1]`
//! `referenced-property-increment` (a REAL). Each extended `parameters` entry
//! is an application-tagged primitive or a `[0]`-framed
//! `BACnetDeviceObjectPropertyReference`. Lists hold at most 10,000 items.
//!
//! Four tags carry no alternative here. Tag 6 is skipped because it would
//! mirror the proprietary complex-event-type notification alternative, and
//! tag 19 is skipped because change-of-reliability has no event parameters.
//! Tag 7 is deprecated and tag 12 is reserved. These four are rejected on
//! decode. Any other unassigned tag (23 and up) is preserved byte-for-byte
//! through [`BACnetEventParameter::Opaque`].

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, ChangeOfValueCriteria,
};
use bacnet_types::error::{DecodeError, DecodingKind, EncodeError, Error};
use bytes::BytesMut;

use crate::primitives;
use crate::tags;

use super::tagged::{
    contents, decode_app_bit_string, decode_ctx_bit_string, decode_ctx_constructed,
    decode_ctx_real, decode_ctx_unsigned, expect_closing, expect_opening, next_is_closing,
};
use super::{
    decode_dopr_body, decode_property_state, encode_dopr_body, encode_property_state,
    validate_extended_parameters, validate_tlv_sequence, MAX_FRAMED_ITEMS,
};

/// Context tags the Clause 21 production omits, deprecates, or reserves.
/// Decoding one means the peer is speaking something other than the 135-2020
/// production, so these are hard errors rather than Opaque shadows.
const RESERVED_EVENT_TAGS: [u8; 4] = [6, 7, 12, 19];

/// The highest tag the 2020 production assigns or reserves: every tag up to
/// it has a variant or is rejected, so none of them is [`BACnetEventParameter::Opaque`].
const LAST_ASSIGNED_TAG: u8 = 22;

/// Encode a [`BACnetEventParameter`] as its full CHOICE framing.
///
/// Validation includes the complete framing depth. Errors leave `buf`
/// unchanged.
pub fn encode_event_parameter(
    buf: &mut BytesMut,
    value: &BACnetEventParameter,
) -> Result<(), Error> {
    let mut encoded = BytesMut::new();
    encode_event_parameter_into(&mut encoded, value)?;
    validate_tlv_sequence(&encoded, "BACnetEventParameter")
        .map_err(Error::into_encode_refusal)?;
    buf.extend_from_slice(&encoded);
    Ok(())
}

fn encode_event_parameter_into(
    buf: &mut BytesMut,
    value: &BACnetEventParameter,
) -> Result<(), Error> {
    match value {
        BACnetEventParameter::ChangeOfBitstring {
            time_delay,
            bitmask,
            list_of_values,
        } => {
            tags::encode_opening_tag(buf, 0);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            primitives::encode_ctx_bit_string(buf, 1, bitmask.0, &bitmask.1);
            tags::encode_opening_tag(buf, 2);
            for (unused_bits, data) in list_of_values {
                primitives::encode_app_bit_string(buf, *unused_bits, data);
            }
            tags::encode_closing_tag(buf, 2);
            tags::encode_closing_tag(buf, 0);
        }
        BACnetEventParameter::ChangeOfState {
            time_delay,
            list_of_values,
        } => {
            let mut encoded_states = BytesMut::new();
            for state in list_of_values {
                encode_property_state(&mut encoded_states, state)?;
            }
            tags::encode_opening_tag(buf, 1);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            tags::encode_opening_tag(buf, 1);
            buf.extend_from_slice(&encoded_states);
            tags::encode_closing_tag(buf, 1);
            tags::encode_closing_tag(buf, 1);
        }
        BACnetEventParameter::ChangeOfValue {
            time_delay,
            criteria,
        } => {
            tags::encode_opening_tag(buf, 2);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            // cov-criteria [1] CHOICE — a tagged CHOICE is always explicitly
            // tagged: opening/closing [1] around the alternative's own tag.
            tags::encode_opening_tag(buf, 1);
            match criteria {
                ChangeOfValueCriteria::Bitmask { unused_bits, data } => {
                    primitives::encode_ctx_bit_string(buf, 0, *unused_bits, data);
                }
                ChangeOfValueCriteria::ReferencedPropertyIncrement(increment) => {
                    primitives::encode_ctx_real(buf, 1, *increment);
                }
            }
            tags::encode_closing_tag(buf, 1);
            tags::encode_closing_tag(buf, 2);
        }
        BACnetEventParameter::FloatingLimit {
            time_delay,
            setpoint_reference,
            low_diff_limit,
            high_diff_limit,
            deadband,
        } => {
            tags::encode_opening_tag(buf, 4);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            // setpoint-reference [1] BACnetDeviceObjectPropertyReference (SEQUENCE)
            tags::encode_opening_tag(buf, 1);
            encode_dopr_body(buf, setpoint_reference);
            tags::encode_closing_tag(buf, 1);
            primitives::encode_ctx_real(buf, 2, *low_diff_limit);
            primitives::encode_ctx_real(buf, 3, *high_diff_limit);
            primitives::encode_ctx_real(buf, 4, *deadband);
            tags::encode_closing_tag(buf, 4);
        }
        BACnetEventParameter::OutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => {
            tags::encode_opening_tag(buf, 5);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            primitives::encode_ctx_real(buf, 1, *low_limit);
            primitives::encode_ctx_real(buf, 2, *high_limit);
            primitives::encode_ctx_real(buf, 3, *deadband);
            tags::encode_closing_tag(buf, 5);
        }
        BACnetEventParameter::Extended {
            vendor_id,
            extended_event_type,
            parameters,
        } => {
            validate_extended_parameters(parameters, "extended")
                .map_err(Error::into_encode_refusal)?;
            tags::encode_opening_tag(buf, 9);
            primitives::encode_ctx_unsigned(buf, 0, *vendor_id as u64);
            primitives::encode_ctx_unsigned(buf, 1, *extended_event_type as u64);
            // parameters [2] SEQUENCE OF CHOICE — held as raw pre-encoded items.
            tags::encode_opening_tag(buf, 2);
            buf.extend_from_slice(parameters);
            tags::encode_closing_tag(buf, 2);
            tags::encode_closing_tag(buf, 9);
        }
        BACnetEventParameter::None => {
            // Clause 21 defines none [20] as context NULL, not an empty SEQUENCE.
            tags::encode_tag(buf, 20, tags::TagClass::Context, 0);
        }
        BACnetEventParameter::Opaque { tag, data } if *tag == u8::MAX => {
            // Legacy pre-framing EventEnrollment values used an Octet String.
            primitives::encode_app_octet_string(buf, data);
        }
        BACnetEventParameter::Opaque { tag, .. } if *tag <= LAST_ASSIGNED_TAG => {
            return Err(
                EncodeError::new("tag [{}] cannot use an opaque constructed body")
                    .within("BACnetEventParameter")
                    .with_value((*tag).into())
                    .into(),
            );
        }
        BACnetEventParameter::Opaque { tag, data } => {
            validate_tlv_sequence(data, "opaque event parameters")
                .map_err(Error::into_encode_refusal)?;
            // Preserved alternative: its captured bytes are the complete
            // SEQUENCE body, re-emitted under its own opening/closing pair.
            tags::encode_opening_tag(buf, *tag);
            buf.extend_from_slice(data);
            tags::encode_closing_tag(buf, *tag);
        }
        BACnetEventParameter::CommandFailure {
            time_delay,
            feedback_property_reference,
        } => {
            tags::encode_opening_tag(buf, 3);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            encode_reference(buf, 1, feedback_property_reference);
            tags::encode_closing_tag(buf, 3);
        }
        BACnetEventParameter::ChangeOfLifeSafety {
            time_delay,
            list_of_life_safety_alarm_values,
            list_of_alarm_values,
            mode_property_reference,
        } => {
            tags::encode_opening_tag(buf, 8);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            encode_enumerations(buf, 1, list_of_life_safety_alarm_values.iter().map(|s| s.to_raw()));
            encode_enumerations(buf, 2, list_of_alarm_values.iter().map(|s| s.to_raw()));
            encode_reference(buf, 3, mode_property_reference);
            tags::encode_closing_tag(buf, 8);
        }
        BACnetEventParameter::BufferReady {
            notification_threshold,
            previous_notification_count,
        } => {
            tags::encode_opening_tag(buf, 10);
            primitives::encode_ctx_unsigned(buf, 0, u64::from(*notification_threshold));
            primitives::encode_ctx_unsigned(buf, 1, u64::from(*previous_notification_count));
            tags::encode_closing_tag(buf, 10);
        }
        BACnetEventParameter::UnsignedRange {
            time_delay,
            low_limit,
            high_limit,
        } => {
            tags::encode_opening_tag(buf, 11);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            primitives::encode_ctx_unsigned(buf, 1, *low_limit);
            primitives::encode_ctx_unsigned(buf, 2, *high_limit);
            tags::encode_closing_tag(buf, 11);
        }
        BACnetEventParameter::AccessEvent {
            list_of_access_events,
            access_event_time_reference,
        } => {
            tags::encode_opening_tag(buf, 13);
            encode_enumerations(buf, 0, list_of_access_events.iter().map(|e| e.to_raw()));
            encode_reference(buf, 1, access_event_time_reference);
            tags::encode_closing_tag(buf, 13);
        }
        BACnetEventParameter::DoubleOutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => {
            tags::encode_opening_tag(buf, 14);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            primitives::encode_ctx_double(buf, 1, *low_limit);
            primitives::encode_ctx_double(buf, 2, *high_limit);
            primitives::encode_ctx_double(buf, 3, *deadband);
            tags::encode_closing_tag(buf, 14);
        }
        BACnetEventParameter::SignedOutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => {
            tags::encode_opening_tag(buf, 15);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            primitives::encode_ctx_signed(buf, 1, *low_limit);
            primitives::encode_ctx_signed(buf, 2, *high_limit);
            primitives::encode_ctx_unsigned(buf, 3, *deadband);
            tags::encode_closing_tag(buf, 15);
        }
        BACnetEventParameter::UnsignedOutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => {
            tags::encode_opening_tag(buf, 16);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            primitives::encode_ctx_unsigned(buf, 1, *low_limit);
            primitives::encode_ctx_unsigned(buf, 2, *high_limit);
            primitives::encode_ctx_unsigned(buf, 3, *deadband);
            tags::encode_closing_tag(buf, 16);
        }
        BACnetEventParameter::ChangeOfCharacterstring {
            time_delay,
            list_of_alarm_values,
        } => {
            let mut strings = BytesMut::new();
            for value in list_of_alarm_values {
                primitives::encode_app_character_string(&mut strings, value)?;
            }
            tags::encode_opening_tag(buf, 17);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            tags::encode_opening_tag(buf, 1);
            buf.extend_from_slice(&strings);
            tags::encode_closing_tag(buf, 1);
            tags::encode_closing_tag(buf, 17);
        }
        BACnetEventParameter::ChangeOfStatusFlags {
            time_delay,
            selected_flags,
        } => {
            tags::encode_opening_tag(buf, 18);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            primitives::encode_ctx_bit_string(buf, 1, 4, &[selected_flags.bits() << 4]);
            tags::encode_closing_tag(buf, 18);
        }
        BACnetEventParameter::ChangeOfDiscreteValue { time_delay } => {
            tags::encode_opening_tag(buf, 21);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            tags::encode_closing_tag(buf, 21);
        }
        BACnetEventParameter::ChangeOfTimer {
            time_delay,
            alarm_values,
            update_time_reference,
        } => {
            tags::encode_opening_tag(buf, 22);
            primitives::encode_ctx_unsigned(buf, 0, *time_delay as u64);
            encode_enumerations(buf, 1, alarm_values.iter().map(|s| s.to_raw()));
            encode_reference(buf, 2, update_time_reference);
            tags::encode_closing_tag(buf, 22);
        }
    }
    Ok(())
}

/// A reference member: its opening tag, the reference's members, its
/// closing tag.
fn encode_reference(buf: &mut BytesMut, tag: u8, reference: &BACnetDeviceObjectPropertyReference) {
    tags::encode_opening_tag(buf, tag);
    encode_dopr_body(buf, reference);
    tags::encode_closing_tag(buf, tag);
}

/// A list of enumerations member: its opening tag, each value application
/// tagged, its closing tag.
fn encode_enumerations(buf: &mut BytesMut, tag: u8, values: impl Iterator<Item = u32>) {
    tags::encode_opening_tag(buf, tag);
    for value in values {
        primitives::encode_app_enumerated(buf, value);
    }
    tags::encode_closing_tag(buf, tag);
}

/// Decode one framed [`BACnetEventParameter`] CHOICE alternative at `offset`.
///
/// Returns the value and the offset past its encoding. An alternative under a
/// tag the 2020 production doesn't assign is preserved as
/// [`BACnetEventParameter::Opaque`]; the omitted/deprecated/reserved tags 6,
/// 7, 12, 19 are rejected.
pub fn decode_event_parameter(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetEventParameter, usize), Error> {
    use BACnetEventParameter as EP;

    // Pre-framing EventEnrollment writes use [255] as an internal wrapper.
    // Keep this shipped compatibility form local so the generic tag parser
    // can reject ASHRAE-reserved tag number 255 everywhere else.
    if data.get(offset..offset.saturating_add(2)) == Some(&[0xFE, 0xFF]) {
        let body_start = offset + 2;
        // The wrapper needs its closing pair after the opening one.
        if data.len() < body_start + 2 {
            return Err(Error::buffer_too_short(body_start + 2, data.len()));
        }
        let body_end = data.len() - 2;
        if data.get(body_end..) != Some(&[0xFF, 0xFF]) {
            return Err(Error::decoding(
                offset,
                "BACnetEventParameter: missing legacy closing tag",
            ));
        }
        return Ok((
            EP::Opaque {
                tag: u8::MAX,
                data: data[body_start..body_end].to_vec(),
            },
            data.len(),
        ));
    }

    let (tag, pos) = tags::decode_tag(data, offset)?;
    let what = "BACnetEventParameter";

    if tag.class == tags::TagClass::Application && tag.number == tags::app_tag::OCTET_STRING {
        let (octets, end) = contents(data, pos, tag.length)?;
        return Ok((
            EP::Opaque {
                tag: u8::MAX,
                data: octets.to_vec(),
            },
            end,
        ));
    }

    if tag.class != tags::TagClass::Context {
        return Err(Error::decoding(
            offset,
            "BACnetEventParameter: expected a context tag",
        ));
    }

    if RESERVED_EVENT_TAGS.contains(&tag.number) {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "BACnetEventParameter: context tag [{}] is omitted/deprecated/reserved in Clause 21",
        )
        .with_value(tag.number.into())
        .into());
    }

    // `none [20] NULL` is the only primitive alternative in this CHOICE.
    if !tag.is_opening && !tag.is_closing {
        if tag.number == 20 && tag.length == 0 {
            return Ok((EP::None, pos));
        }
        return Err(Error::decoding(
            offset,
            "BACnetEventParameter: invalid primitive alternative",
        ));
    }

    if !tag.is_opening {
        return Err(Error::decoding(
            offset,
            "BACnetEventParameter: unexpected closing tag",
        ));
    }
    if tag.number == 20 {
        return Err(Error::decoding(
            offset,
            "BACnetEventParameter: none [20] must use primitive NULL form",
        ));
    }

    let value = match tag.number {
        0 => {
            let what = "BACnetEventParameter change-of-bitstring";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let (bitmask, pos) = decode_ctx_bit_string(data, pos, 1, what)?;
            let mut pos = expect_opening(data, pos, 2, what)?;
            let mut list_of_values = Vec::new();
            while !next_is_closing(data, pos, 2)? {
                if list_of_values.len() >= MAX_FRAMED_ITEMS {
                    return Err(Error::overflow(
                        pos,
                        "change-of-bitstring: list-of-bitstring-values exceeds limit",
                    ));
                }
                let (bits, p) = decode_app_bit_string(data, pos, what)?;
                list_of_values.push(bits);
                pos = p;
            }
            pos = expect_closing(data, pos, 2, what)?;
            pos = expect_closing(data, pos, 0, what)?;
            (
                EP::ChangeOfBitstring {
                    time_delay,
                    bitmask,
                    list_of_values,
                },
                pos,
            )
        }
        1 => {
            let what = "BACnetEventParameter change-of-state";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let mut pos = expect_opening(data, pos, 1, what)?;
            let mut list_of_values = Vec::new();
            while !next_is_closing(data, pos, 1)? {
                if list_of_values.len() >= MAX_FRAMED_ITEMS {
                    return Err(Error::overflow(
                        pos,
                        "change-of-state: list-of-values exceeds limit",
                    ));
                }
                let (state, p) = decode_property_state(data, pos)?;
                list_of_values.push(state);
                pos = p;
            }
            pos = expect_closing(data, pos, 1, what)?;
            pos = expect_closing(data, pos, 1, what)?;
            (
                EP::ChangeOfState {
                    time_delay,
                    list_of_values,
                },
                pos,
            )
        }
        2 => {
            let what = "BACnetEventParameter change-of-value";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let mut pos = expect_opening(data, pos, 1, what)?;
            let (inner, _) = tags::decode_tag(data, pos)?;
            let criteria = if inner.is_context(0) {
                let (bits, p) = decode_ctx_bit_string(data, pos, 0, what)?;
                pos = p;
                ChangeOfValueCriteria::Bitmask {
                    unused_bits: bits.0,
                    data: bits.1,
                }
            } else if inner.is_context(1) {
                let (increment, p) = decode_ctx_real(data, pos, 1, what)?;
                pos = p;
                ChangeOfValueCriteria::ReferencedPropertyIncrement(increment)
            } else {
                return Err(DecodeError::new(
                    DecodingKind::InvalidEncoding,
                    pos,
                    "change-of-value: unexpected cov-criteria tag [{}]",
                )
                .with_value(inner.number.into())
                .into());
            };
            pos = expect_closing(data, pos, 1, what)?;
            pos = expect_closing(data, pos, 2, what)?;
            (
                EP::ChangeOfValue {
                    time_delay,
                    criteria,
                },
                pos,
            )
        }
        4 => {
            let what = "BACnetEventParameter floating-limit";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let pos = expect_opening(data, pos, 1, what)?;
            let (setpoint_reference, pos) = decode_dopr_body(data, pos, what)?;
            let pos = expect_closing(data, pos, 1, what)?;
            let (low_diff_limit, pos) = decode_ctx_real(data, pos, 2, what)?;
            let (high_diff_limit, pos) = decode_ctx_real(data, pos, 3, what)?;
            let (deadband, pos) = decode_ctx_real(data, pos, 4, what)?;
            let pos = expect_closing(data, pos, 4, what)?;
            (
                EP::FloatingLimit {
                    time_delay,
                    setpoint_reference,
                    low_diff_limit,
                    high_diff_limit,
                    deadband,
                },
                pos,
            )
        }
        5 => {
            let what = "BACnetEventParameter out-of-range";
            let (time_delay, pos) = decode_ctx_unsigned::<u32>(data, pos, 0, what)?;
            let (low_limit, pos) = decode_ctx_real(data, pos, 1, what)?;
            let (high_limit, pos) = decode_ctx_real(data, pos, 2, what)?;
            let (deadband, pos) = decode_ctx_real(data, pos, 3, what)?;
            let pos = expect_closing(data, pos, 5, what)?;
            (
                EP::OutOfRange {
                    time_delay,
                    low_limit,
                    high_limit,
                    deadband,
                },
                pos,
            )
        }
        9 => {
            let what = "BACnetEventParameter extended";
            let (vendor_id, pos) = decode_ctx_unsigned::<u16>(data, pos, 0, what)?;
            let (extended_event_type, pos) = decode_ctx_unsigned::<u32>(data, pos, 1, what)?;
            let (raw_params, pos) = decode_ctx_constructed(data, pos, 2, what)?;
            validate_extended_parameters(raw_params, "extended")?;
            let pos = expect_closing(data, pos, 9, what)?;
            (
                EP::Extended {
                    vendor_id,
                    extended_event_type,
                    parameters: raw_params.to_vec(),
                },
                pos,
            )
        }
        n @ (3 | 8 | 10 | 11 | 13..=18 | 21 | 22) => members::decode(data, pos, n)?,
        n => {
            // A tag the 2020 production doesn't assign: preserve the SEQUENCE
            // body verbatim after validating its nested tag structure.
            let (body, p) = tags::extract_context_value(data, pos, n)?;
            validate_tlv_sequence(body, "opaque event parameters")?;
            (
                EP::Opaque {
                    tag: n,
                    data: body.to_vec(),
                },
                p,
            )
        }
    };
    let (value, end) = value;
    validate_tlv_sequence(&data[offset..end], what)?;
    Ok((value, end))
}

mod members;
