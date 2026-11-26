//! Decoding an acknowledgement and checking it answers its request: the
//! answer checks of the typed calls' service descriptions (#2588), which
//! every way of sending a request shares.

use bacnet_services::file::{AtomicReadFileAck, FileAccessMethod, FileReadAckMethod};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::read_range::{
    ReadRangeAck, ReadRangeReply, ReadRangeRequest, ReadRangeValidation,
};
use bacnet_services::rpm::ReadPropertyMultipleACK;
use bacnet_types::{enums::ObjectType, error::Error, primitives::ObjectIdentifier};

/// Only a short read against the complete service payload establishes its end offset.
fn decode<T>(
    bytes: &[u8],
    service: &'static str,
    parser: impl FnOnce(&[u8]) -> Result<T, Error>,
) -> Result<T, Error> {
    parser(bytes).map_err(|err| match err {
        Error::BufferTooShort { need, have } if have == bytes.len() && need > have => {
            Error::Decoding(err.to_decode_error(have).within(service))
        }
        other => other,
    })
}

/// Decode the complete ReadPropertyMultiple ACK before accessing individual rows.
pub(crate) fn read_property_multiple(bytes: &[u8]) -> Result<ReadPropertyMultipleACK, Error> {
    decode(
        bytes,
        "ReadPropertyMultiple ACK",
        ReadPropertyMultipleACK::decode,
    )
}

/// Decode the ReadRange ACK, then check it against the original request.
pub(crate) fn read_range(
    request: &ReadRangeRequest,
    bytes: &[u8],
    validation: ReadRangeValidation,
) -> Result<ReadRangeReply, Error> {
    let ack = decode(bytes, "ReadRange ACK", ReadRangeAck::decode)?;
    ReadRangeReply::check(request, ack, validation)
}

/// A ReadProperty acknowledgement for `request`: it must name the object,
/// property and array index read (Clause 15.5). A request by the Device or
/// Network Port wildcard instance takes the object of that type the device
/// reports as its own; a wildcard instance in the answer is refused.
///
/// # Errors
///
/// A decoding error for an answer that doesn't decode or doesn't match.
pub(crate) fn read_property(
    request: &ReadPropertyRequest,
    bytes: &[u8],
) -> Result<ReadPropertyACK, Error> {
    let ack = decode(bytes, "ReadProperty ACK", ReadPropertyACK::decode)?;
    let requested = request.object_identifier;
    let reported = ack.object_identifier;
    // 15.5.2 permits these request aliases; 15.5.1.2 identifies the object
    // actually read in the ACK. A wildcard ACK never establishes that identity.
    let permitted_alias = requested.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE
        && matches!(
            requested.object_type(),
            ObjectType::DEVICE | ObjectType::NETWORK_PORT
        );
    if reported.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE
        || reported.object_type() != requested.object_type()
        || (!permitted_alias && reported != requested)
    {
        return Err(Error::decoding(
            0,
            "ReadProperty ACK object identifier does not match the request",
        ));
    }
    if ack.property_identifier != request.property_identifier {
        return Err(Error::decoding(
            0,
            "ReadProperty ACK property identifier does not match the request",
        ));
    }
    if ack.property_array_index != request.property_array_index {
        return Err(Error::decoding(
            0,
            "ReadProperty ACK array index does not match the request",
        ));
    }
    Ok(ack)
}

/// An AtomicReadFile acknowledgement for a request with `access`: the same
/// access method, and no more octets or records than it asked for.
///
/// # Errors
///
/// A decoding error for an answer that doesn't decode or doesn't match.
pub(crate) fn atomic_read_file(
    access: &FileAccessMethod,
    bytes: &[u8],
) -> Result<AtomicReadFileAck, Error> {
    let ack = decode(bytes, "AtomicReadFile ACK", AtomicReadFileAck::decode)?;
    validate_atomic_read_file_ack(access, &ack)?;
    Ok(ack)
}

/// The window check of [`atomic_read_file`], on a decoded answer.
pub(crate) fn validate_atomic_read_file_ack(
    request: &FileAccessMethod,
    ack: &AtomicReadFileAck,
) -> Result<(), Error> {
    match (request, &ack.access) {
        (
            FileAccessMethod::Stream {
                requested_octet_count,
                ..
            },
            FileReadAckMethod::Stream { file_data, .. },
        ) if file_data.len() <= *requested_octet_count as usize => Ok(()),
        (FileAccessMethod::Stream { .. }, FileReadAckMethod::Stream { .. }) => Err(
            Error::decoding(0, "AtomicReadFile ACK exceeds the requested octet window"),
        ),
        (
            FileAccessMethod::Record {
                requested_record_count,
                ..
            },
            FileReadAckMethod::Record {
                returned_record_count,
                file_record_data,
                ..
            },
        ) if returned_record_count <= requested_record_count
            && *returned_record_count as usize == file_record_data.len() =>
        {
            Ok(())
        }
        (FileAccessMethod::Record { .. }, FileReadAckMethod::Record { .. }) => Err(
            Error::decoding(0, "AtomicReadFile ACK exceeds the requested record window"),
        ),
        _ => Err(Error::decoding(
            0,
            "AtomicReadFile ACK access method does not match the request",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::error::{DecodeError, DecodingKind};

    #[test]
    fn whole_ack_guard_preserves_slice_sizes_and_existing_errors() {
        let bytes = [0u8; 4];
        for (need, have) in [(3, 2), (4, 4), (3, 4), (7, 6)] {
            let result = decode::<()>(&bytes, "test ACK", |_| {
                Err(Error::BufferTooShort { need, have })
            });
            assert!(
                matches!(result, Err(Error::BufferTooShort { need: n, have: h }) if (n,h) == (need,have))
            );
        }
        let inner = DecodeError::new(DecodingKind::InvalidTag, 1, "inner tag {}")
            .within("field")
            .at_item(2)
            .with_value(7);
        assert!(
            matches!(decode::<()>(&bytes, "test ACK", |_| Err(inner.into())),
                         Err(Error::Decoding(fault)) if fault == inner)
        );
        assert!(
            matches!(decode::<()>(&bytes, "test ACK", |_| Err(Error::invalid_argument("request"))),
                         Err(Error::InvalidArgument(reason)) if reason.to_string() == "request")
        );
    }

    #[test]
    fn complete_file_ack_retains_inner_decoder_offset_and_details() {
        let bytes = [0x11, 0x0e, 0x31, 0, 0x0f];
        let access = FileAccessMethod::Stream {
            file_start_position: 0,
            requested_octet_count: 3,
        };
        let Error::Decoding(inner) = AtomicReadFileAck::decode(&bytes).unwrap_err() else {
            panic!("expected the stream's missing file-data diagnostic");
        };
        assert_eq!((inner.kind, inner.offset), (DecodingKind::Missing, 2));
        assert!(
            matches!(atomic_read_file(&access, &bytes), Err(Error::Decoding(fault)) if fault == inner)
        );
    }
}
