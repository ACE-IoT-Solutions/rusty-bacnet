use super::*;

impl RuntimeTransport {
    pub(crate) fn background_task_count(&self) -> usize {
        match self {
            Self::Bip(_) => 2,
            #[cfg(feature = "mstp")]
            Self::Mstp(_) => 2,
            #[cfg(feature = "sc")]
            Self::Sc(_) => 2,
        }
    }

    pub(crate) fn health_error(&self) -> Option<crate::ErrorCode> {
        match self {
            Self::Bip(_) => None,
            #[cfg(feature = "mstp")]
            Self::Mstp(_) => None,
            #[cfg(feature = "sc")]
            Self::Sc(_) => None,
        }
    }

    pub(crate) fn foreign_device_registration_status(
        &self,
    ) -> Option<ForeignDeviceRegistrationStatus> {
        match self {
            Self::Bip(_) => None,
            #[cfg(feature = "mstp")]
            Self::Mstp(_) => None,
            #[cfg(feature = "sc")]
            Self::Sc(_) => None,
        }
    }

    pub(crate) async fn read_rpm(
        &self,
        attachment_id: crate::AttachmentId,
        path: &DevicePath,
        batch: &RpmBatch,
    ) -> Result<bacnet_services::rpm::ReadPropertyMultipleACK, RuntimeError> {
        let result = match (self, path) {
            (Self::Bip(client), DevicePath::Direct { mac }) => {
                client
                    .read_property_multiple(mac, batch.specifications.clone())
                    .await
            }
            #[cfg(feature = "mstp")]
            (Self::Mstp(client), DevicePath::Direct { mac }) => {
                client
                    .read_property_multiple(mac, batch.specifications.clone())
                    .await
            }
            #[cfg(feature = "sc")]
            (Self::Sc(client), DevicePath::Direct { mac }) => {
                client
                    .read_property_multiple(mac, batch.specifications.clone())
                    .await
            }
            (
                Self::Bip(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .read_property_multiple_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        batch.specifications.clone(),
                    )
                    .await
            }
            #[cfg(feature = "mstp")]
            (
                Self::Mstp(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .read_property_multiple_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        batch.specifications.clone(),
                    )
                    .await
            }
            #[cfg(feature = "sc")]
            (
                Self::Sc(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .read_property_multiple_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        batch.specifications.clone(),
                    )
                    .await
            }
        };
        result.map_err(|error| RuntimeError::operation(attachment_id, error))
    }

    pub(crate) async fn read_one(
        &self,
        attachment_id: crate::AttachmentId,
        path: &DevicePath,
        read: &PropertyRead,
    ) -> Result<Vec<u8>, RuntimeError> {
        let object = ObjectIdentifier::new(
            ObjectType::from_raw(u32::from(read.object_type)),
            read.object_instance,
        )
        .map_err(|error| RuntimeError::invalid_config(error.to_string()))?;
        let property = PropertyIdentifier::from_raw(read.property_id);
        let result = match (self, path) {
            (Self::Bip(client), DevicePath::Direct { mac }) => {
                client
                    .read_property(mac, object, property, read.array_index)
                    .await
            }
            #[cfg(feature = "mstp")]
            (Self::Mstp(client), DevicePath::Direct { mac }) => {
                client
                    .read_property(mac, object, property, read.array_index)
                    .await
            }
            #[cfg(feature = "sc")]
            (Self::Sc(client), DevicePath::Direct { mac }) => {
                client
                    .read_property(mac, object, property, read.array_index)
                    .await
            }
            (
                Self::Bip(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .read_property_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        object,
                        property,
                        read.array_index,
                    )
                    .await
            }
            #[cfg(feature = "mstp")]
            (
                Self::Mstp(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .read_property_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        object,
                        property,
                        read.array_index,
                    )
                    .await
            }
            #[cfg(feature = "sc")]
            (
                Self::Sc(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .read_property_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        object,
                        property,
                        read.array_index,
                    )
                    .await
            }
        };
        result
            .map(|ack| ack.property_value)
            .map_err(|error| RuntimeError::operation(attachment_id, error))
    }

    pub(crate) async fn write_one(
        &self,
        attachment_id: crate::AttachmentId,
        path: &DevicePath,
        target: &PropertyRead,
        raw_value: Vec<u8>,
        priority: Option<u8>,
    ) -> Result<(), RuntimeError> {
        let object = ObjectIdentifier::new(
            ObjectType::from_raw(u32::from(target.object_type)),
            target.object_instance,
        )
        .map_err(|error| RuntimeError::invalid_config(error.to_string()))?;
        let property = PropertyIdentifier::from_raw(target.property_id);
        let result = match (self, path) {
            (Self::Bip(client), DevicePath::Direct { mac }) => {
                client
                    .write_property(
                        mac,
                        object,
                        property,
                        target.array_index,
                        raw_value,
                        priority,
                    )
                    .await
            }
            #[cfg(feature = "mstp")]
            (Self::Mstp(client), DevicePath::Direct { mac }) => {
                client
                    .write_property(
                        mac,
                        object,
                        property,
                        target.array_index,
                        raw_value,
                        priority,
                    )
                    .await
            }
            #[cfg(feature = "sc")]
            (Self::Sc(client), DevicePath::Direct { mac }) => {
                client
                    .write_property(
                        mac,
                        object,
                        property,
                        target.array_index,
                        raw_value,
                        priority,
                    )
                    .await
            }
            (
                Self::Bip(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .write_property_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        object,
                        property,
                        target.array_index,
                        raw_value,
                        priority,
                    )
                    .await
            }
            #[cfg(feature = "mstp")]
            (
                Self::Mstp(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .write_property_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        object,
                        property,
                        target.array_index,
                        raw_value,
                        priority,
                    )
                    .await
            }
            #[cfg(feature = "sc")]
            (
                Self::Sc(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .write_property_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        object,
                        property,
                        target.array_index,
                        raw_value,
                        priority,
                    )
                    .await
            }
        };
        result.map_err(|error| RuntimeError::operation(attachment_id, error))
    }

    pub(crate) async fn write_multiple(
        &self,
        attachment_id: crate::AttachmentId,
        path: &DevicePath,
        writes: &[(PropertyRead, Vec<u8>, Option<u8>)],
    ) -> Result<(), RuntimeError> {
        let mut specs: Vec<WriteAccessSpecification> = Vec::new();
        for (target, raw_value, priority) in writes {
            let object = ObjectIdentifier::new(
                ObjectType::from_raw(u32::from(target.object_type)),
                target.object_instance,
            )
            .map_err(|error| RuntimeError::invalid_config(error.to_string()))?;
            let property = BACnetPropertyValue {
                property_identifier: PropertyIdentifier::from_raw(target.property_id),
                property_array_index: target.array_index,
                value: raw_value.clone(),
                priority: *priority,
            };
            if let Some(spec) = specs
                .iter_mut()
                .find(|spec| spec.object_identifier == object)
            {
                spec.list_of_properties.push(property);
            } else {
                specs.push(WriteAccessSpecification {
                    object_identifier: object,
                    list_of_properties: vec![property],
                });
            }
        }
        let result = match (self, path) {
            (Self::Bip(client), DevicePath::Direct { mac }) => {
                client.write_property_multiple(mac, specs).await
            }
            #[cfg(feature = "mstp")]
            (Self::Mstp(client), DevicePath::Direct { mac }) => {
                client.write_property_multiple(mac, specs).await
            }
            #[cfg(feature = "sc")]
            (Self::Sc(client), DevicePath::Direct { mac }) => {
                client.write_property_multiple(mac, specs).await
            }
            (
                Self::Bip(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .write_property_multiple_routed(ingress_mac, *dnet, dadr, specs)
                    .await
            }
            #[cfg(feature = "mstp")]
            (
                Self::Mstp(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .write_property_multiple_routed(ingress_mac, *dnet, dadr, specs)
                    .await
            }
            #[cfg(feature = "sc")]
            (
                Self::Sc(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .write_property_multiple_routed(ingress_mac, *dnet, dadr, specs)
                    .await
            }
        };
        result.map_err(|error| RuntimeError::operation(attachment_id, error))
    }

    pub(crate) async fn subscribe_cov(
        &self,
        attachment_id: crate::AttachmentId,
        path: &DevicePath,
        spec: &crate::ObservationSpec,
    ) -> Result<(), RuntimeError> {
        let object = ObjectIdentifier::new(
            ObjectType::from_raw(u32::from(spec.key.object_type)),
            spec.key.object_instance,
        )
        .map_err(|error| RuntimeError::invalid_config(error.to_string()))?;
        let result = match (self, path) {
            (Self::Bip(client), DevicePath::Direct { mac }) => {
                client
                    .subscribe_cov(
                        mac,
                        spec.key.subscriber_process_id,
                        object,
                        spec.confirmed,
                        Some(spec.lifetime_seconds),
                    )
                    .await
            }
            #[cfg(feature = "mstp")]
            (Self::Mstp(client), DevicePath::Direct { mac }) => {
                client
                    .subscribe_cov(
                        mac,
                        spec.key.subscriber_process_id,
                        object,
                        spec.confirmed,
                        Some(spec.lifetime_seconds),
                    )
                    .await
            }
            #[cfg(feature = "sc")]
            (Self::Sc(client), DevicePath::Direct { mac }) => {
                client
                    .subscribe_cov(
                        mac,
                        spec.key.subscriber_process_id,
                        object,
                        spec.confirmed,
                        Some(spec.lifetime_seconds),
                    )
                    .await
            }
            (
                Self::Bip(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .subscribe_cov_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        spec.key.subscriber_process_id,
                        object,
                        spec.confirmed,
                        Some(spec.lifetime_seconds),
                    )
                    .await
            }
            #[cfg(feature = "mstp")]
            (
                Self::Mstp(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .subscribe_cov_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        spec.key.subscriber_process_id,
                        object,
                        spec.confirmed,
                        Some(spec.lifetime_seconds),
                    )
                    .await
            }
            #[cfg(feature = "sc")]
            (
                Self::Sc(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .subscribe_cov_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        spec.key.subscriber_process_id,
                        object,
                        spec.confirmed,
                        Some(spec.lifetime_seconds),
                    )
                    .await
            }
        };
        result.map_err(|error| RuntimeError::operation(attachment_id, error))
    }

    pub(crate) async fn unsubscribe_cov(
        &self,
        attachment_id: crate::AttachmentId,
        path: &DevicePath,
        spec: &crate::ObservationSpec,
    ) -> Result<(), RuntimeError> {
        let object = ObjectIdentifier::new(
            ObjectType::from_raw(u32::from(spec.key.object_type)),
            spec.key.object_instance,
        )
        .map_err(|error| RuntimeError::invalid_config(error.to_string()))?;
        let result = match (self, path) {
            (Self::Bip(client), DevicePath::Direct { mac }) => {
                client
                    .unsubscribe_cov(mac, spec.key.subscriber_process_id, object)
                    .await
            }
            #[cfg(feature = "mstp")]
            (Self::Mstp(client), DevicePath::Direct { mac }) => {
                client
                    .unsubscribe_cov(mac, spec.key.subscriber_process_id, object)
                    .await
            }
            #[cfg(feature = "sc")]
            (Self::Sc(client), DevicePath::Direct { mac }) => {
                client
                    .unsubscribe_cov(mac, spec.key.subscriber_process_id, object)
                    .await
            }
            (
                Self::Bip(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .unsubscribe_cov_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        spec.key.subscriber_process_id,
                        object,
                    )
                    .await
            }
            #[cfg(feature = "mstp")]
            (
                Self::Mstp(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .unsubscribe_cov_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        spec.key.subscriber_process_id,
                        object,
                    )
                    .await
            }
            #[cfg(feature = "sc")]
            (
                Self::Sc(client),
                DevicePath::Routed {
                    ingress_mac,
                    dnet,
                    dadr,
                },
            ) => {
                client
                    .unsubscribe_cov_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        spec.key.subscriber_process_id,
                        object,
                    )
                    .await
            }
        };
        result.map_err(|error| RuntimeError::operation(attachment_id, error))
    }

    pub(crate) fn cov_receiver(&self) -> broadcast::Receiver<ReceivedCOVNotification> {
        match self {
            Self::Bip(client) => client.cov_notifications(),
            #[cfg(feature = "mstp")]
            Self::Mstp(client) => client.cov_notifications(),
            #[cfg(feature = "sc")]
            Self::Sc(client) => client.cov_notifications(),
        }
    }

    pub(crate) fn take_initial_cov_receiver(
        &mut self,
    ) -> Option<broadcast::Receiver<ReceivedCOVNotification>> {
        Some(self.cov_receiver())
    }

    pub(crate) fn i_am_receiver(&self) -> broadcast::Receiver<IAmEvent> {
        match self {
            Self::Bip(client) => client.iam_events(),
            #[cfg(feature = "mstp")]
            Self::Mstp(client) => client.iam_events(),
            #[cfg(feature = "sc")]
            Self::Sc(client) => client.iam_events(),
        }
    }

    pub(crate) fn take_initial_i_am_receiver(&mut self) -> Option<broadcast::Receiver<IAmEvent>> {
        match self {
            Self::Bip(client) => client.take_initial_iam_receiver(),
            #[cfg(feature = "mstp")]
            Self::Mstp(client) => client.take_initial_iam_receiver(),
            #[cfg(feature = "sc")]
            Self::Sc(client) => client.take_initial_iam_receiver(),
        }
    }
}
