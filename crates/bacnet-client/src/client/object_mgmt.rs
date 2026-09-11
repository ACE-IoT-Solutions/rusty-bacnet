use super::*;

impl<T: TransportPort + 'static> BACnetClient<T> {
    async fn delete_object_target(
        &self,
        target: ConfirmedTarget<'_>,
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
    ) -> Result<(), Error> {
        use bacnet_services::object_mgmt::DeleteObjectRequest;

        let request = DeleteObjectRequest { object_identifier };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        self.confirmed_request_inner(target, ConfirmedServiceChoice::DELETE_OBJECT, &buf)
            .await?;
        Ok(())
    }

    pub async fn delete_object(
        &self,
        destination_mac: &[u8],
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
    ) -> Result<(), Error> {
        self.delete_object_target(
            ConfirmedTarget::Local {
                mac: destination_mac,
            },
            object_identifier,
        )
        .await
    }

    /// Delete an object through an explicit BACnet router.
    pub async fn delete_object_routed(
        &self,
        router_mac: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
    ) -> Result<(), Error> {
        self.delete_object_target(
            ConfirmedTarget::Routed {
                router_mac,
                dest_network,
                dest_mac,
            },
            object_identifier,
        )
        .await
    }

    async fn create_object_target(
        &self,
        target: ConfirmedTarget<'_>,
        object_specifier: bacnet_services::object_mgmt::ObjectSpecifier,
        initial_values: Vec<bacnet_services::common::BACnetPropertyValue>,
    ) -> Result<Bytes, Error> {
        use bacnet_services::object_mgmt::CreateObjectRequest;

        let request = CreateObjectRequest {
            object_specifier,
            list_of_initial_values: initial_values,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        self.confirmed_request_inner(target, ConfirmedServiceChoice::CREATE_OBJECT, &buf)
            .await
    }

    /// Create an object on a remote device.
    pub async fn create_object(
        &self,
        destination_mac: &[u8],
        object_specifier: bacnet_services::object_mgmt::ObjectSpecifier,
        initial_values: Vec<bacnet_services::common::BACnetPropertyValue>,
    ) -> Result<Bytes, Error> {
        self.create_object_target(
            ConfirmedTarget::Local {
                mac: destination_mac,
            },
            object_specifier,
            initial_values,
        )
        .await
    }

    /// Create an object through an explicit BACnet router.
    pub async fn create_object_routed(
        &self,
        router_mac: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        object_specifier: bacnet_services::object_mgmt::ObjectSpecifier,
        initial_values: Vec<bacnet_services::common::BACnetPropertyValue>,
    ) -> Result<Bytes, Error> {
        self.create_object_target(
            ConfirmedTarget::Routed {
                router_mac,
                dest_network,
                dest_mac,
            },
            object_specifier,
            initial_values,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_encoding::{
        apdu::{self, Apdu, ComplexAck},
        npdu::decode_npdu,
    };
    use bacnet_services::{common::BACnetPropertyValue, object_mgmt::CreateObjectRequest};
    use bacnet_transport::{loopback::LoopbackTransport, port::TransportPort};
    use bacnet_types::{
        enums::{ObjectType, PropertyIdentifier},
        primitives::ObjectIdentifier,
    };
    use bytes::Bytes;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn routed_create_preserves_target_initial_value_order_indexes_and_priorities() {
        let client_mac = vec![0x31];
        let router_mac = vec![0x32];
        let remote_network = 0x4567;
        let remote_mac = vec![0xde, 0xad, 0xbe, 0xef];
        let object_id = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 77).unwrap();
        let initial_values = vec![
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: Some(3),
                value: vec![0x21, 0x2a],
                priority: Some(8),
            },
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::DESCRIPTION,
                property_array_index: None,
                value: vec![0x21, 0x07],
                priority: None,
            },
        ];
        let expected_values = initial_values.clone();
        let (client_transport, mut router_transport) =
            LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
        let mut router_rx = router_transport.start().await.unwrap();
        let mut client = BACnetClient::generic_builder()
            .transport(client_transport)
            .apdu_timeout_ms(1_000)
            .build()
            .await
            .unwrap();

        let request_router = router_mac.clone();
        let request_remote = remote_mac.clone();
        let request = tokio::spawn(async move {
            let result = client
                .create_object_routed(
                    &request_router,
                    remote_network,
                    &request_remote,
                    bacnet_services::object_mgmt::ObjectSpecifier::Identifier(object_id),
                    initial_values,
                )
                .await;
            client.stop().await.unwrap();
            result
        });

        let received = timeout(Duration::from_secs(2), router_rx.recv())
            .await
            .expect("router timed out")
            .expect("router channel closed");
        assert_eq!(&received.source_mac[..], &client_mac);
        let npdu = decode_npdu(received.npdu).unwrap();
        let destination = npdu.destination.expect("routed destination");
        assert_eq!(destination.network, remote_network);
        assert_eq!(&destination.mac_address[..], &remote_mac);
        let Apdu::ConfirmedRequest(confirmed) = apdu::decode_apdu(npdu.payload).unwrap() else {
            panic!("expected confirmed request")
        };
        assert_eq!(
            confirmed.service_choice,
            ConfirmedServiceChoice::CREATE_OBJECT
        );
        let decoded = CreateObjectRequest::decode(&confirmed.service_request).unwrap();
        assert_eq!(
            decoded.object_specifier,
            bacnet_services::object_mgmt::ObjectSpecifier::Identifier(object_id)
        );
        assert_eq!(decoded.list_of_initial_values, expected_values);

        crate::client::tests::send_routed_response(
            &router_transport,
            &client_mac,
            remote_network,
            &remote_mac,
            Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: confirmed.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::CREATE_OBJECT,
                service_ack: Bytes::from_static(&[0xc4, 0x00, 0x80, 0x00, 0x4d]),
            }),
        )
        .await;

        assert!(request.await.unwrap().is_ok());
        router_transport.stop().await.unwrap();
    }
}
