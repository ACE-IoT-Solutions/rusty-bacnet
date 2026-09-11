use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::{COVNotificationDelivery, ReceivedCOVNotification};
use bacnet_encoding::apdu::{
    self, encode_apdu, Apdu, ComplexAck, RejectPdu, SimpleAck, UnconfirmedRequest,
};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_encoding::primitives::encode_app_object_id;
use bacnet_network::layer::NetworkLayer;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::{COVNotificationRequest, SubscribeCOVRequest};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::rpm::{ReadAccessResult, ReadPropertyMultipleACK, ReadResultElement};
use bacnet_services::who_is::IAmRequest;
use bacnet_services::wpm::WritePropertyMultipleRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::bbmd::BdtEntry;
use bacnet_transport::bip::{BipTransport, ForeignDeviceConfig};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{
    BvlcFunction, ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier,
    RejectReason, Segmentation, UnconfirmedServiceChoice,
};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::time::timeout;

use crate::{
    AttachmentConfig, AttachmentId, BipConfig, DeviceKey, DeviceObservation, DevicePath, ErrorCode,
    EventKind, FreshnessPolicy, MstpConfig, ObservationKey, ObservationPlan, ObservationSpec,
    PlanLimits, PropertyRead, ReadBatch, ReadBatchItem, ReadSource, RuntimeConfig, ScanRequest,
    Support, TransportConfig, WorkPriority, WriteBatch, WriteBatchItem,
};

use super::BacnetRuntime;

include!("tests/group_1.rs");
include!("tests/group_2.rs");
include!("tests/group_3.rs");
include!("tests/group_4.rs");
include!("tests/group_5.rs");
include!("tests/group_6.rs");
include!("tests/group_7.rs");
