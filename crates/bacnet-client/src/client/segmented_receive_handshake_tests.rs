//! Window negotiation with a peer that waits for SegmentACK 0.

use bacnet_encoding::apdu::{Apdu, ComplexAck};
use bacnet_encoding::constructed::encode_log_record;
use bacnet_encoding::primitives::encode_app_octet_string;
use bacnet_services::file::{AtomicReadFileAck, FileAccessMethod, FileReadAckMethod};
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest, ReadRangeValidation};
use bacnet_services::rpm::{ReadAccessResult, ReadPropertyMultipleACK, ReadResultElement};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::constructed::{
    BACnetLogRecord, LogDatum, PropertyReference, ReadAccessSpecification,
};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::error::ReadRangeViolation;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use super::segmented_receive_lifecycle_tests::{
    CLIENT_MAC, SERVER_MAC, expect_segment_ack, recv_apdu, send_to_client,
};
use super::{BACnetClient, ClientConfig};

async fn start() -> (
    BACnetClient<LoopbackTransport>,
    LoopbackTransport,
    mpsc::Receiver<ReceivedNpdu>,
) {
    let (transport, mut server) = LoopbackTransport::pair(CLIENT_MAC.to_vec(), SERVER_MAC.to_vec());
    let rx = server.start().await.unwrap();
    let client = BACnetClient::start(ClientConfig::default(), transport)
        .await
        .unwrap();
    (client, server, rx)
}

fn property_value() -> Vec<u8> {
    let data: Vec<_> = (0..=255).cycle().take(1800).collect();
    let mut encoded = BytesMut::new();
    encode_app_octet_string(&mut encoded, &data);
    encoded.to_vec()
}

async fn send_windowed_response(
    server: LoopbackTransport,
    mut rx: mpsc::Receiver<ReceivedNpdu>,
    service: ConfirmedServiceChoice,
    payload: BytesMut,
) {
    let Apdu::ConfirmedRequest(request) = recv_apdu(&mut rx, "confirmed request").await else {
        panic!("expected a confirmed request");
    };
    assert_eq!(request.service_choice, service);
    assert!(request.segmented_response_accepted);
    assert!(request.max_apdu_length >= 485);

    // The first three APDUs carry 480 service bytes and a five-byte segment header.
    let segments = payload.chunks(480);
    assert_eq!(segments.len(), 4);
    for (seq, data) in (0u8..).zip(segments) {
        send_to_client(
            &server,
            &Apdu::ComplexAck(ComplexAck {
                segmented: true,
                more_follows: seq < 3,
                invoke_id: request.invoke_id,
                sequence_number: Some(seq),
                proposed_window_size: Some(2),
                service_choice: service,
                service_ack: Bytes::copy_from_slice(data),
            }),
        )
        .await;
        // Wait after segment zero, then after the two-segment window, then the final segment.
        if matches!(seq, 0 | 2 | 3) {
            assert_eq!(expect_segment_ack(&mut rx, request.invoke_id, seq).await, 2);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn read_property_negotiates_window_before_segment_one() {
    let (mut client, server, rx) = start().await;
    let object = ObjectIdentifier::new(ObjectType::OCTETSTRING_VALUE, 1).unwrap();
    let property = PropertyIdentifier::PRESENT_VALUE;
    let expected = ReadPropertyACK {
        object_identifier: object,
        property_identifier: property,
        property_array_index: None,
        property_value: property_value(),
    };
    let mut payload = BytesMut::new();
    expected.encode(&mut payload).unwrap();
    let (result, ()) = tokio::join!(
        client.read_property(SERVER_MAC, object, property, None),
        send_windowed_response(server, rx, ConfirmedServiceChoice::READ_PROPERTY, payload)
    );
    assert_eq!(result.unwrap(), expected);
    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn read_property_multiple_negotiates_window_before_segment_one() {
    let (mut client, server, rx) = start().await;
    let object = ObjectIdentifier::new(ObjectType::OCTETSTRING_VALUE, 1).unwrap();
    let property = PropertyIdentifier::PRESENT_VALUE;
    let spec = ReadAccessSpecification {
        object_identifier: object,
        list_of_property_references: vec![PropertyReference {
            property_identifier: property,
            property_array_index: None,
        }],
    };
    let expected = ReadPropertyMultipleACK {
        list_of_read_access_results: vec![ReadAccessResult {
            object_identifier: object,
            list_of_results: vec![ReadResultElement {
                property_identifier: property,
                property_array_index: None,
                property_value: Some(property_value()),
                error: None,
            }],
        }],
    };
    let mut payload = BytesMut::new();
    expected.encode(&mut payload).unwrap();
    let (result, ()) = tokio::join!(
        client.read_property_multiple(SERVER_MAC, vec![spec]),
        send_windowed_response(
            server,
            rx,
            ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
            payload
        )
    );
    assert_eq!(result.unwrap(), expected);
    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn read_range_preserves_segmented_trend_page_with_sequence_zero() {
    let (mut client, server, rx) = start().await;
    let request = ReadRangeRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(RangeSpec::BySequenceNumber {
            reference_seq: 0,
            count: 2,
        }),
    };
    let record = BACnetLogRecord {
        date: Date {
            year: 126,
            month: 11,
            day: 24,
            day_of_week: 2,
        },
        time: Time {
            hour: 12,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        log_datum: LogDatum::AnyValue(property_value()),
        status_flags: None,
    };
    let mut item_data = BytesMut::new();
    encode_log_record(&record, &mut item_data).unwrap();
    let expected = ReadRangeAck {
        object_identifier: request.object_identifier,
        property_identifier: request.property_identifier,
        property_array_index: None,
        result_flags: (true, false, true),
        item_count: 1,
        item_data: item_data.to_vec(),
        first_sequence_number: Some(0),
    };
    let mut payload = BytesMut::new();
    expected.encode(&mut payload);
    let (result, ()) = tokio::join!(
        client.read_range_with(SERVER_MAC, &request, ReadRangeValidation::Lenient),
        send_windowed_response(server, rx, ConfirmedServiceChoice::READ_RANGE, payload)
    );
    let reply = result.unwrap();
    assert_eq!(
        reply.violations,
        [ReadRangeViolation::ZeroFirstSequenceNumber]
    );
    assert_eq!(reply.ack.result_flags, expected.result_flags);
    assert_eq!(reply.ack.first_sequence_number, Some(0));
    assert_eq!(reply.ack.item_count, 1);
    assert_eq!(reply.ack.item_data, expected.item_data);
    assert_eq!(reply.ack.trend_log_records().unwrap(), [record]);
    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn atomic_read_file_decodes_segmented_final_stream_window() {
    let (mut client, server, rx) = start().await;
    let file = ObjectIdentifier::new(ObjectType::FILE, 1).unwrap();
    let window = FileAccessMethod::Stream {
        file_start_position: 3072,
        requested_octet_count: 2048,
    };
    let expected = AtomicReadFileAck {
        end_of_file: true,
        access: FileReadAckMethod::Stream {
            file_start_position: 3072,
            file_data: (0..=255).cycle().take(1800).collect(),
        },
    };
    let mut payload = BytesMut::new();
    expected.encode(&mut payload);
    let (result, ()) = tokio::join!(
        client.atomic_read_file_decoded(SERVER_MAC, file, window),
        send_windowed_response(
            server,
            rx,
            ConfirmedServiceChoice::ATOMIC_READ_FILE,
            payload
        )
    );
    assert_eq!(result.unwrap(), expected);
    client.stop().await.unwrap();
}
