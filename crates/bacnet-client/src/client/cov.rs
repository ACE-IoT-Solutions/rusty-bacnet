use super::*;
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

impl<T: TransportPort + 'static> BACnetClient<T> {
    fn subscribe_cov_request(
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
        confirmed: Option<bool>,
        lifetime: Option<u32>,
    ) -> SubscribeCOVRequest {
        SubscribeCOVRequest {
            subscriber_process_identifier,
            monitored_object_identifier,
            issue_confirmed_notifications: confirmed,
            lifetime,
        }
    }

    async fn send_subscribe_cov_request(
        &self,
        target: ConfirmedTarget<'_>,
        request: SubscribeCOVRequest,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let _ = self
            .confirmed_request_inner(target, ConfirmedServiceChoice::SUBSCRIBE_COV, &buf)
            .await?;

        Ok(())
    }

    fn subscribe_cov_property_request(
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
        confirmed: Option<bool>,
        lifetime: Option<u32>,
        monitored_property_identifier: PropertyIdentifier,
        monitored_property_array_index: Option<u32>,
        cov_increment: Option<f32>,
    ) -> SubscribeCOVPropertyRequest {
        SubscribeCOVPropertyRequest {
            subscriber_process_identifier,
            monitored_object_identifier,
            issue_confirmed_notifications: confirmed,
            lifetime,
            monitored_property_identifier,
            monitored_property_array_index,
            cov_increment,
        }
    }

    async fn send_subscribe_cov_property_request(
        &self,
        target: ConfirmedTarget<'_>,
        request: SubscribeCOVPropertyRequest,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let _ = self
            .confirmed_request_inner(target, ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, &buf)
            .await?;

        Ok(())
    }

    async fn send_subscribe_cov_property_multiple_request(
        &self,
        target: ConfirmedTarget<'_>,
        request: &SubscribeCOVPropertyMultipleRequest,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        request.try_encode(&mut buf)?;
        self.confirmed_request_inner(
            target,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            &buf,
        )
        .await?;
        Ok(())
    }

    /// Send SubscribeCOVPropertyMultiple to a directly reachable device.
    pub async fn subscribe_cov_property_multiple(
        &self,
        destination_mac: &[u8],
        request: &SubscribeCOVPropertyMultipleRequest,
    ) -> Result<(), Error> {
        self.send_subscribe_cov_property_multiple_request(
            ConfirmedTarget::Local {
                mac: destination_mac,
            },
            request,
        )
        .await
    }

    /// Send SubscribeCOVPropertyMultiple through an explicit BACnet router.
    pub async fn subscribe_cov_property_multiple_routed(
        &self,
        router_mac: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        request: &SubscribeCOVPropertyMultipleRequest,
    ) -> Result<(), Error> {
        self.send_subscribe_cov_property_multiple_request(
            ConfirmedTarget::Routed {
                router_mac,
                dest_network,
                dest_mac,
            },
            request,
        )
        .await
    }

    /// Subscribe to COV notifications for an object at a directly reachable MAC address.
    pub async fn subscribe_cov(
        &self,
        destination_mac: &[u8],
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> Result<(), Error> {
        let request = Self::subscribe_cov_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            Some(confirmed),
            lifetime,
        );

        self.send_subscribe_cov_request(
            ConfirmedTarget::Local {
                mac: destination_mac,
            },
            request,
        )
        .await
    }

    /// Subscribe to COV notifications for an object on a remote network via a router.
    #[allow(clippy::too_many_arguments)]
    pub async fn subscribe_cov_routed(
        &self,
        router_mac: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> Result<(), Error> {
        let request = Self::subscribe_cov_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            Some(confirmed),
            lifetime,
        );

        self.send_subscribe_cov_request(
            ConfirmedTarget::Routed {
                router_mac,
                dest_network,
                dest_mac,
            },
            request,
        )
        .await
    }

    /// Subscribe to COV notifications for a discovered device, auto-routing if needed.
    pub async fn subscribe_cov_to_device(
        &self,
        device_instance: u32,
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> Result<(), Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        let request = Self::subscribe_cov_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            Some(confirmed),
            lifetime,
        );

        if let Some((dnet, dadr)) = routing {
            self.send_subscribe_cov_request(
                ConfirmedTarget::Routed {
                    router_mac: &mac,
                    dest_network: dnet,
                    dest_mac: &dadr,
                },
                request,
            )
            .await
        } else {
            self.send_subscribe_cov_request(ConfirmedTarget::Local { mac: &mac }, request)
                .await
        }
    }

    /// Cancel a COV subscription on a remote device.
    pub async fn unsubscribe_cov(
        &self,
        destination_mac: &[u8],
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
    ) -> Result<(), Error> {
        let request = Self::subscribe_cov_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            None,
            None,
        );

        self.send_subscribe_cov_request(
            ConfirmedTarget::Local {
                mac: destination_mac,
            },
            request,
        )
        .await
    }

    /// Cancel a COV subscription on a remote network via a router.
    pub async fn unsubscribe_cov_routed(
        &self,
        router_mac: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
    ) -> Result<(), Error> {
        let request = Self::subscribe_cov_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            None,
            None,
        );

        self.send_subscribe_cov_request(
            ConfirmedTarget::Routed {
                router_mac,
                dest_network,
                dest_mac,
            },
            request,
        )
        .await
    }

    /// Cancel a COV subscription for a discovered device, auto-routing if needed.
    pub async fn unsubscribe_cov_to_device(
        &self,
        device_instance: u32,
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
    ) -> Result<(), Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        let request = Self::subscribe_cov_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            None,
            None,
        );

        if let Some((dnet, dadr)) = routing {
            self.send_subscribe_cov_request(
                ConfirmedTarget::Routed {
                    router_mac: &mac,
                    dest_network: dnet,
                    dest_mac: &dadr,
                },
                request,
            )
            .await
        } else {
            self.send_subscribe_cov_request(ConfirmedTarget::Local { mac: &mac }, request)
                .await
        }
    }

    /// Subscribe to COV notifications for a single property at a directly reachable MAC address.
    pub async fn subscribe_cov_property(
        &self,
        destination_mac: &[u8],
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
        monitored_property_identifier: PropertyIdentifier,
        monitored_property_array_index: Option<u32>,
        confirmed: bool,
        lifetime: Option<u32>,
        cov_increment: Option<f32>,
    ) -> Result<(), Error> {
        let request = Self::subscribe_cov_property_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            Some(confirmed),
            lifetime,
            monitored_property_identifier,
            monitored_property_array_index,
            cov_increment,
        );

        self.send_subscribe_cov_property_request(
            ConfirmedTarget::Local {
                mac: destination_mac,
            },
            request,
        )
        .await
    }

    /// Subscribe to COV notifications for a single property on a discovered device,
    /// auto-routing if needed.
    pub async fn subscribe_cov_property_to_device(
        &self,
        device_instance: u32,
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
        monitored_property_identifier: PropertyIdentifier,
        monitored_property_array_index: Option<u32>,
        confirmed: bool,
        lifetime: Option<u32>,
        cov_increment: Option<f32>,
    ) -> Result<(), Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        let request = Self::subscribe_cov_property_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            Some(confirmed),
            lifetime,
            monitored_property_identifier,
            monitored_property_array_index,
            cov_increment,
        );

        if let Some((dnet, dadr)) = routing {
            self.send_subscribe_cov_property_request(
                ConfirmedTarget::Routed {
                    router_mac: &mac,
                    dest_network: dnet,
                    dest_mac: &dadr,
                },
                request,
            )
            .await
        } else {
            self.send_subscribe_cov_property_request(ConfirmedTarget::Local { mac: &mac }, request)
                .await
        }
    }

    /// Cancel a single-property COV subscription at a directly reachable MAC address.
    pub async fn unsubscribe_cov_property(
        &self,
        destination_mac: &[u8],
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
        monitored_property_identifier: PropertyIdentifier,
        monitored_property_array_index: Option<u32>,
    ) -> Result<(), Error> {
        let request = Self::subscribe_cov_property_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            None,
            None,
            monitored_property_identifier,
            monitored_property_array_index,
            None,
        );

        self.send_subscribe_cov_property_request(
            ConfirmedTarget::Local {
                mac: destination_mac,
            },
            request,
        )
        .await
    }

    /// Cancel a single-property COV subscription for a discovered device,
    /// auto-routing if needed.
    pub async fn unsubscribe_cov_property_to_device(
        &self,
        device_instance: u32,
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
        monitored_property_identifier: PropertyIdentifier,
        monitored_property_array_index: Option<u32>,
    ) -> Result<(), Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        let request = Self::subscribe_cov_property_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            None,
            None,
            monitored_property_identifier,
            monitored_property_array_index,
            None,
        );

        if let Some((dnet, dadr)) = routing {
            self.send_subscribe_cov_property_request(
                ConfirmedTarget::Routed {
                    router_mac: &mac,
                    dest_network: dnet,
                    dest_mac: &dadr,
                },
                request,
            )
            .await
        } else {
            self.send_subscribe_cov_property_request(ConfirmedTarget::Local { mac: &mac }, request)
                .await
        }
    }

    /// Get a receiver for incoming COV notifications. Each call returns a new
    /// independent receiver.
    pub fn cov_notifications(&self) -> broadcast::Receiver<ReceivedCOVNotification> {
        self.cov_tx.subscribe()
    }

    /// Take the startup-reserved COV receiver, if it has not already been taken.
    ///
    /// Aggregate runtimes use this once immediately after client construction
    /// to retain notifications dispatched before their background pump starts.
    pub fn take_initial_cov_receiver(
        &mut self,
    ) -> Option<broadcast::Receiver<ReceivedCOVNotification>> {
        self.initial_cov_rx.take()
    }
}
