//! An incoming COV transfer and an outbound read can share an invoke ID.
use super::*;
use bacnet_encoding::apdu::{AbortPdu, RejectPdu};
use bacnet_services::who_is::IAmRequest;
use bacnet_types::enums::{AbortReason, EnableDisable, RejectReason, Segmentation};

fn deliver(wire: &Wire, source: &MacAddr, apdu: &Apdu) {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, apdu).unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply: matches!(apdu, Apdu::ConfirmedRequest(_)),
            payload: payload.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    wire.deliver(&Wire::mac(NODE), wire::frame(source, npdu.freeze(), false));
}

fn parts(invoke_id: u8) -> [Apdu; 2] {
    let encoded = node::cov_notification(invoke_id);
    let Apdu::ConfirmedRequest(base) = decode_apdu(decode_npdu(encoded).unwrap().payload).unwrap()
    else {
        unreachable!()
    };
    [0u8, 1].map(|seq| {
        Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: true,
            more_follows: seq == 0,
            sequence_number: Some(seq),
            proposed_window_size: Some(2),
            service_request: if seq == 0 {
                base.service_request.slice(..8)
            } else {
                base.service_request.slice(8..)
            },
            ..base.clone()
        })
    })
}

async fn wait_for(log: &SendLog, peer: &MacAddr, want: impl Fn(&Apdu) -> bool) {
    eventually(WAIT, "an answer from the node", async || {
        sent_to(log, peer).iter().any(&want)
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn requesters_abort_leaves_the_nodes_outbound_read_alive_and_dcc_allows_replies() {
    let wire = Wire::new();
    let port = wire.port(NODE);
    let log = port.sent();
    let mut session =
        EndpointSession::new(port.build(), SessionRole::Both, SessionConfig::default())
            .unwrap()
            .device(
                DeviceIdentity::new(NODE_DEVICE, 42)
                    .unwrap()
                    .with_segmentation(Segmentation::SEGMENTED_RECEIVE),
            )
            .with_server(
                BACnetServer::generic_builder()
                    .dcc_policy(bacnet_server::server::DccPolicy::LegacyPermissive),
            );
    session.start().await.unwrap();
    let client = session.full_client().unwrap();
    let mut events = client.cov_notifications();
    session.broadcast_i_am().await.unwrap();
    assert!(
        sent_to(&log, &MacAddr::new())
            .iter()
            .any(|apdu| matches!(apdu,
        Apdu::UnconfirmedRequest(pdu) if IAmRequest::decode(&pdu.service_request)
            .is_ok_and(|iam| iam.segmentation_supported == Segmentation::SEGMENTED_RECEIVE)))
    );

    let peer = Wire::mac(PEER);
    let read = client.read_property(&peer, ai1(), PropertyIdentifier::PRESENT_VALUE, None);
    let requester = async {
        eventually(WAIT, "the node's outstanding read", async || {
            !requests(&log, ConfirmedServiceChoice::READ_PROPERTY).is_empty()
        })
        .await;
        let id = requests(&log, ConfirmedServiceChoice::READ_PROPERTY)[0];
        let [first, last] = parts(id);
        deliver(&wire, &peer, &first);
        wait_for(&log, &peer, |apdu| matches!(apdu, Apdu::SegmentAck(ack) if ack.sent_by_server && ack.invoke_id == id && ack.sequence_number == 0)).await;
        deliver(
            &wire,
            &peer,
            &Apdu::Abort(AbortPdu {
                sent_by_server: false,
                invoke_id: id,
                abort_reason: AbortReason::OTHER,
            }),
        );
        deliver(&wire, &peer, &last);
        wait_for(&log, &peer, |apdu| matches!(apdu, Apdu::Abort(abort) if abort.sent_by_server && abort.invoke_id == id && abort.abort_reason == AbortReason::INVALID_APDU_IN_THIS_STATE)).await;
        assert!(events.try_recv().is_err());
        deliver(
            &wire,
            &peer,
            &Apdu::Reject(RejectPdu {
                invoke_id: id,
                reject_reason: RejectReason::UNRECOGNIZED_SERVICE,
            }),
        );
    };
    let (result, ()) = tokio::join!(read, requester);
    assert_eq!(
        result.unwrap_err().reject_reason(),
        Some(RejectReason::UNRECOGNIZED_SERVICE)
    );

    let mut operator = BACnetClient::generic_builder()
        .transport(wire.port(OTHER).build())
        .build()
        .await
        .unwrap();
    operator
        .device_communication_control(
            &Wire::mac(NODE),
            EnableDisable::DISABLE_INITIATION,
            None,
            None,
        )
        .await
        .unwrap();
    let [first, last] = parts(82);
    deliver(&wire, &peer, &first);
    wait_for(&log, &peer, |apdu| matches!(apdu, Apdu::SegmentAck(ack) if ack.invoke_id == 82 && ack.sequence_number == 0)).await;
    deliver(&wire, &peer, &last);
    wait_for(
        &log,
        &peer,
        |apdu| matches!(apdu, Apdu::SimpleAck(ack) if ack.invoke_id == 82),
    )
    .await;
    assert_eq!(
        events.try_recv().unwrap().subscriber_process_identifier(),
        1
    );
    drop(client);
    session.stop().await.unwrap();
    operator.stop().await.unwrap();
}
