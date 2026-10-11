//! Borrowed routed-NPDU view and rewrite helpers.

use super::{
    rejection::RefusalSource, ForwardError, LinkAddress, NetworkNumber, BACNET_PROTOCOL_VERSION,
    DESTINATION_BIT, NETWORK_MESSAGE_BIT, SOURCE_BIT,
};

pub(super) struct ParsedNpdu<'a> {
    pub(super) control: u8,
    pub(super) destination_network: u16,
    pub(super) destination_mac: &'a [u8],
    source_fields: Option<&'a [u8]>,
    source_return: Option<RefusalSource>,
    hop_count: u8,
    remainder: &'a [u8],
}

impl<'a> ParsedNpdu<'a> {
    pub(super) fn decode(data: &'a [u8]) -> Result<Self, ForwardError> {
        Self::decode_inner(data, true)
    }

    /// Decode enough structure to identify a refusal source, including a
    /// packet whose hop count is already exhausted. Forwarding still uses
    /// [`Self::decode`] and therefore rejects that packet before egress.
    pub(super) fn decode_for_refusal(data: &'a [u8]) -> Result<Self, ForwardError> {
        Self::decode_inner(data, false)
    }

    fn decode_inner(data: &'a [u8], reject_exhausted_hop: bool) -> Result<Self, ForwardError> {
        if data.len() < 2 {
            return Err(ForwardError::Truncated);
        }
        if data[0] != BACNET_PROTOCOL_VERSION {
            return Err(ForwardError::UnsupportedVersion(data[0]));
        }
        let control = data[1];
        if control & DESTINATION_BIT == 0 {
            return Err(ForwardError::MissingDestination);
        }
        let mut offset = 2;
        if data.len() < offset + 3 {
            return Err(ForwardError::Truncated);
        }
        let destination_network = u16::from_be_bytes([data[offset], data[offset + 1]]);
        offset += 2;
        let destination_len = data[offset] as usize;
        offset += 1;
        if destination_len > 6 || data.len() < offset + destination_len + 1 {
            return Err(if destination_len > 6 {
                ForwardError::AddressTooLong
            } else {
                ForwardError::Truncated
            });
        }
        let destination_mac = &data[offset..offset + destination_len];
        offset += destination_len;

        let (source_fields, source_return) = if control & SOURCE_BIT != 0 {
            let source_start = offset;
            if data.len() < offset + 3 {
                return Err(ForwardError::Truncated);
            }
            let source_network_value = u16::from_be_bytes([data[offset], data[offset + 1]]);
            offset += 2;
            let source_len = data[offset] as usize;
            offset += 1;
            if source_network_value == 0 || source_network_value == u16::MAX || source_len == 0 {
                return Err(ForwardError::InvalidSource);
            }
            if source_len > 6 {
                return Err(ForwardError::AddressTooLong);
            }
            if data.len() < offset + source_len + 1 {
                return Err(ForwardError::Truncated);
            }
            let source_network = NetworkNumber::new(source_network_value)
                .map_err(|_| ForwardError::InvalidSource)?;
            let source_address = LinkAddress::try_from_slice(&data[offset..offset + source_len])
                .map_err(|_| ForwardError::AddressTooLong)?;
            offset += source_len;
            (
                Some(&data[source_start..offset]),
                Some(RefusalSource::Routed {
                    network: source_network,
                    address: source_address,
                }),
            )
        } else {
            (None, None)
        };

        // Skip Hop Count. The direct egress removes it with DNET/DADR.
        let hop_count = data[offset];
        if reject_exhausted_hop && hop_count == 0 {
            return Err(ForwardError::HopExhausted);
        }
        offset += 1;
        if control & NETWORK_MESSAGE_BIT != 0 {
            let Some(&message_type) = data.get(offset) else {
                return Err(ForwardError::InvalidNetworkMessage);
            };
            if message_type >= 0x80 && data.len() < offset + 3 {
                return Err(ForwardError::InvalidNetworkMessage);
            }
        }
        Ok(Self {
            control,
            destination_network,
            destination_mac,
            source_fields,
            source_return,
            hop_count,
            remainder: &data[offset..],
        })
    }

    pub(super) fn refusal_source(
        &self,
        ingress_network: NetworkNumber,
        ingress_source: LinkAddress,
    ) -> Option<RefusalSource> {
        self.source_return.or_else(|| {
            if ingress_source.is_empty() {
                None
            } else {
                Some(RefusalSource::Ingress {
                    network: ingress_network,
                    address: ingress_source,
                })
            }
        })
    }

    pub(super) fn rewrite_direct(
        &self,
        ingress_network: NetworkNumber,
        ingress_source: LinkAddress,
        output: &mut [u8],
    ) -> Result<usize, ForwardError> {
        let needed = self.rewritten_direct_len(ingress_source)?;
        if output.len() < needed {
            return Err(ForwardError::ScratchTooSmall {
                needed,
                available: output.len(),
            });
        }
        output[0] = BACNET_PROTOCOL_VERSION;
        output[1] = (self.control & !DESTINATION_BIT) | SOURCE_BIT;
        let mut offset = 2;
        if let Some(source_fields) = self.source_fields {
            output[offset..offset + source_fields.len()].copy_from_slice(source_fields);
            offset += source_fields.len();
        } else {
            output[offset..offset + 2].copy_from_slice(&ingress_network.get().to_be_bytes());
            offset += 2;
            output[offset] = ingress_source.as_slice().len() as u8;
            offset += 1;
            output[offset..offset + ingress_source.as_slice().len()]
                .copy_from_slice(ingress_source.as_slice());
            offset += ingress_source.as_slice().len();
        }
        output[offset..needed].copy_from_slice(self.remainder);
        Ok(needed)
    }

    pub(super) fn rewritten_direct_len(
        &self,
        ingress_source: LinkAddress,
    ) -> Result<usize, ForwardError> {
        if self.source_fields.is_none() && ingress_source.is_empty() {
            return Err(ForwardError::InvalidSource);
        }
        let source_len = self
            .source_fields
            .map_or(3 + ingress_source.as_slice().len(), <[u8]>::len);
        Ok(2 + source_len + self.remainder.len())
    }

    pub(super) fn rewrite_learned(
        &self,
        ingress_network: NetworkNumber,
        ingress_source: LinkAddress,
        next_hop: LinkAddress,
        output: &mut [u8],
    ) -> Result<usize, ForwardError> {
        let needed = self.rewritten_learned_len(ingress_source, next_hop)?;
        if output.len() < needed {
            return Err(ForwardError::ScratchTooSmall {
                needed,
                available: output.len(),
            });
        }
        output[0] = BACNET_PROTOCOL_VERSION;
        output[1] = self.control | SOURCE_BIT;
        let mut offset = 2;
        output[offset..offset + 2].copy_from_slice(&self.destination_network.to_be_bytes());
        offset += 2;
        output[offset] = next_hop.as_slice().len() as u8;
        offset += 1;
        output[offset..offset + next_hop.as_slice().len()].copy_from_slice(next_hop.as_slice());
        offset += next_hop.as_slice().len();
        if let Some(source_fields) = self.source_fields {
            output[offset..offset + source_fields.len()].copy_from_slice(source_fields);
            offset += source_fields.len();
        } else {
            output[offset..offset + 2].copy_from_slice(&ingress_network.get().to_be_bytes());
            offset += 2;
            output[offset] = ingress_source.as_slice().len() as u8;
            offset += 1;
            output[offset..offset + ingress_source.as_slice().len()]
                .copy_from_slice(ingress_source.as_slice());
            offset += ingress_source.as_slice().len();
        }
        output[offset] = self.hop_count - 1;
        offset += 1;
        output[offset..needed].copy_from_slice(self.remainder);
        Ok(needed)
    }

    pub(super) fn rewritten_learned_len(
        &self,
        ingress_source: LinkAddress,
        next_hop: LinkAddress,
    ) -> Result<usize, ForwardError> {
        if self.source_fields.is_none() && ingress_source.is_empty() {
            return Err(ForwardError::InvalidSource);
        }
        let source_len = self
            .source_fields
            .map_or(3 + ingress_source.as_slice().len(), <[u8]>::len);
        Ok(2 + 2 + 1 + next_hop.as_slice().len() + source_len + 1 + self.remainder.len())
    }
}
