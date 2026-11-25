//! Closed set of endpoint reads sharing transaction and source-reporting ownership.
use super::*;
use bacnet_services::read_range::{ReadRangeReply, ReadRangeRequest, ReadRangeValidation};
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};

/// Read identity, validated when preparing the endpoint transaction.
#[doc(hidden)]
#[derive(Clone)]
pub enum EndpointReadRequest {
    Property(ReadPropertyRequest),
    Range(ReadRangeRequest, ReadRangeValidation),
    Multiple(ReadPropertyMultipleRequest),
}

/// Typed caller payload; source Audit records never retain these values.
#[doc(hidden)]
#[derive(Debug)]
pub enum EndpointReadAck {
    Property(ReadPropertyACK),
    Range(ReadRangeReply),
    Multiple(ReadPropertyMultipleACK),
}

impl EndpointReadRequest {
    /// Validate the endpoint's bounded explicit-reference profile before admission.
    #[doc(hidden)]
    pub fn validate(&self) -> Result<(), Error> {
        if let Self::Multiple(request) = self {
            crate::endpoint_rpm::validate_request(request)?;
        }
        Ok(())
    }

    /// Ordered attempted identities, including duplicate occurrences.
    #[doc(hidden)]
    pub fn identities(&self) -> Vec<(ObjectIdentifier, PropertyIdentifier, Option<u32>)> {
        match self {
            Self::Property(r) => vec![(
                r.object_identifier,
                r.property_identifier,
                r.property_array_index,
            )],
            Self::Range(r, _) => vec![(
                r.object_identifier,
                r.property_identifier,
                r.property_array_index,
            )],
            Self::Multiple(r) => r
                .list_of_read_access_specs
                .iter()
                .flat_map(|s| {
                    s.list_of_property_references.iter().map(|p| {
                        (
                            s.object_identifier,
                            p.property_identifier,
                            p.property_array_index,
                        )
                    })
                })
                .collect(),
        }
    }

    pub(super) fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
        match self {
            Self::Property(r) => {
                r.encode(buf);
                Ok(())
            }
            Self::Range(r, _) => r.encode(buf),
            Self::Multiple(r) => r.encode(buf),
        }
    }

    pub(super) fn service(&self) -> ConfirmedServiceChoice {
        match self {
            Self::Property(_) => ConfirmedServiceChoice::READ_PROPERTY,
            Self::Range(..) => ConfirmedServiceChoice::READ_RANGE,
            Self::Multiple(_) => ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        }
    }

    pub(super) fn decode(&self, bytes: &[u8]) -> Result<EndpointReadAck, Error> {
        match self {
            Self::Property(request) => {
                crate::acks::read_property(request, bytes).map(EndpointReadAck::Property)
            }
            Self::Multiple(request) => {
                crate::endpoint_rpm::decode_ack(request, bytes).map(EndpointReadAck::Multiple)
            }
            Self::Range(request, validation) => {
                crate::acks::read_range(request, bytes, *validation).map(EndpointReadAck::Range)
            }
        }
    }
}

impl EndpointReadAck {
    /// Ordered value-free outcomes after complete ACK correlation.
    #[doc(hidden)]
    pub fn audit_results(&self) -> Vec<(ObjectIdentifier, Option<(ErrorClass, ErrorCode)>)> {
        match self {
            Self::Property(ack) => vec![(ack.object_identifier, None)],
            Self::Range(reply) => vec![(reply.ack.object_identifier, None)],
            Self::Multiple(ack) => ack
                .list_of_read_access_results
                .iter()
                .flat_map(|s| {
                    s.list_of_results
                        .iter()
                        .map(|p| (s.object_identifier, p.error))
                })
                .collect(),
        }
    }

    /// Unique Device identity established by successful results in this operation.
    #[doc(hidden)]
    pub fn target_device(&self) -> Option<ObjectIdentifier> {
        let mut devices = self
            .audit_results()
            .into_iter()
            .filter_map(|(object, error)| {
                (error.is_none()
                    && object.object_type() == ObjectType::DEVICE
                    && object.instance_number() != ObjectIdentifier::WILDCARD_INSTANCE)
                    .then_some(object)
            });
        let first = devices.next()?;
        devices.all(|object| object == first).then_some(first)
    }

    #[doc(hidden)]
    pub fn into_multiple(self) -> Result<ReadPropertyMultipleACK, Error> {
        match self {
            Self::Multiple(ack) => Ok(ack),
            _ => Err(Error::Encoding("endpoint read result kind mismatch".into())),
        }
    }

    #[doc(hidden)]
    pub fn into_property(self) -> Result<ReadPropertyACK, Error> {
        match self {
            Self::Property(ack) => Ok(ack),
            _ => Err(Error::Encoding("endpoint read result kind mismatch".into())),
        }
    }

    #[doc(hidden)]
    pub fn into_range(self) -> Result<ReadRangeReply, Error> {
        match self {
            Self::Range(reply) => Ok(reply),
            _ => Err(Error::Encoding("endpoint read result kind mismatch".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
    use bacnet_types::error::DecodingKind;

    #[test]
    fn bounded_endpoint_read_profile_preserves_whole_ack_truncation() {
        let object = ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 1).unwrap();
        let property = PropertyIdentifier::PRESENT_VALUE;
        let rp = EndpointReadRequest::Property(ReadPropertyRequest {
            object_identifier: object,
            property_identifier: property,
            property_array_index: None,
        });
        let rpm = EndpointReadRequest::Multiple(ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: object,
                list_of_property_references: [property, PropertyIdentifier::DESCRIPTION]
                    .map(|property_identifier| PropertyReference {
                        property_identifier,
                        property_array_index: None,
                    })
                    .to_vec(),
            }],
        });
        let range = EndpointReadRequest::Range(
            ReadRangeRequest {
                object_identifier: ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap(),
                property_identifier: PropertyIdentifier::LOG_BUFFER,
                property_array_index: None,
                range: None,
            },
            ReadRangeValidation::Strict,
        );
        for (request, bytes, need, context) in [
            (rp, &[0x0c, 0, 0][..], 5, "ReadProperty ACK"),
            (
                rpm,
                &[
                    0x0c, 4, 0xc0, 0, 1, 0x1e, 0x29, 0x55, 0x4e, 0x21, 7, 0x4f, 0x29, 0x1c, 0x5e,
                    0x92, 0,
                ],
                18,
                "ReadPropertyMultiple ACK",
            ),
            (
                range,
                &[0x0c, 5, 0, 0, 1, 0x19, 0x83, 0x3a, 5],
                10,
                "ReadRange ACK",
            ),
        ] {
            request.validate().unwrap();
            let Error::Decoding(fault) = request.decode(bytes).unwrap_err() else {
                panic!("expected structured truncation of {context}");
            };
            assert_eq!(fault.kind, DecodingKind::Missing);
            assert_eq!(
                (fault.offset, fault.context, fault.values),
                (bytes.len(), context, [need, bytes.len() as u64])
            );
        }
    }
}
