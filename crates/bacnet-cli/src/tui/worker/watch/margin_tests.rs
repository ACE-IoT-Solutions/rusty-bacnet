//! The renewal margin covers a wait at the device's gate (#1791): a renewal
//! queued behind a request that takes a whole budget still goes early
//! enough for its own retries to finish before the lifetime ends.

use std::sync::Mutex;

use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::Bytes;
use tokio::time::Instant;

use super::*;

const CLIENT: [u8; 1] = [0x41];
const DEVICE: [u8; 1] = [0x42];

#[tokio::test(start_paused = true)]
async fn a_renewal_behind_a_full_budget_request_lands_before_expiry() {
    let (client_end, mut device) = LoopbackTransport::pair(CLIENT.to_vec(), DEVICE.to_vec());
    let mut rx = device.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(client_end)
            .apdu_timeout_ms(2_000)
            .build()
            .await
            .unwrap(),
    );
    // The device acknowledges every SubscribeCOV at once and notes when
    // each arrived.
    let arrivals = Arc::new(Mutex::new(Vec::new()));
    let peer = tokio::spawn({
        let arrivals = Arc::clone(&arrivals);
        async move {
            while let Some(received) = rx.recv().await {
                let npdu = decode_npdu(received.npdu).unwrap();
                let apdu = &npdu.payload;
                if apdu.len() < 4 || apdu[0] >> 4 != 0 {
                    continue;
                }
                arrivals.lock().unwrap().push(Instant::now());
                let ack = [1, 0, 0x20, apdu[2], apdu[3]];
                device
                    .send_unicast(&Bytes::copy_from_slice(&ack), &CLIENT)
                    .await
                    .unwrap();
            }
            device
        }
    });

    let budget = Duration::from_secs(24);
    let lifetime = Lifetime::new(120, budget);
    let rate = RequestLimits::new(0);
    let point = Point {
        device: 100,
        network: None,
        object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        property: PropertyIdentifier::PRESENT_VALUE,
    };
    let start = Instant::now();
    let managed = subscribe(
        &client,
        &rate,
        &CovTarget::local(&DEVICE),
        1,
        point,
        lifetime,
    )
    .await
    .unwrap();
    let due = start + Duration::from_secs(120) - lifetime.margin;

    // Another request to the device takes the gate just before the renewal
    // falls due and holds it for a whole budget.
    tokio::time::sleep_until(due - Duration::from_secs(1)).await;
    let held = rate.turn(point.device).await;
    tokio::time::sleep(budget).await;
    drop(held);
    tokio::time::sleep(Duration::from_secs(1)).await;

    let renewed = arrivals.lock().unwrap().get(1).copied().expect("renewed");
    let expiry = start + Duration::from_secs(120);
    assert!(
        renewed + budget <= expiry,
        "renewal at {:?} leaves less than a budget before expiry",
        renewed - start
    );

    managed.stop().await;
    let Ok(mut client) = Arc::try_unwrap(client) else {
        panic!("client still retained");
    };
    client.stop().await.unwrap();
    peer.abort();
}
