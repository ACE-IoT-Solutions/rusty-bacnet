//! The shell's `register` arms renewal only for a registration the BBMD
//! accepted, and refuses a TTL it couldn't renew (#1822).

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;

use super::*;
use crate::session::renewal_interval;

/// A stand-in BBMD on loopback that answers every Register-Foreign-Device
/// with success, or with its NAK when `refuse` is set; it counts requests.
async fn mock_bbmd(refuse: bool) -> (SocketAddr, Arc<AtomicUsize>) {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let at = socket.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&requests);
    tokio::spawn(async move {
        let mut packet = [0; 64];
        while let Ok((len, from)) = socket.recv_from(&mut packet).await {
            if len < 2 || packet[1] != 0x05 {
                continue;
            }
            counted.fetch_add(1, Ordering::SeqCst);
            let code = if refuse { 0x30 } else { 0x00 };
            let _ = socket
                .send_to(&[0x81, 0x00, 0x00, 0x06, 0x00, code], from)
                .await;
        }
    });
    (at, requests)
}

async fn client() -> Arc<BACnetClient<BipTransport>> {
    let client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .build()
        .await
        .unwrap();
    Arc::new(client)
}

async fn register(client: &Arc<BACnetClient<BipTransport>>, args: &[String]) -> Session {
    let mut session = Session::new();
    handle_bip_register(client, args, &mut session, OutputFormat::Table).await;
    session
}

fn args(bbmd: SocketAddr, ttl: &str) -> Vec<String> {
    vec![bbmd.to_string(), "--ttl".into(), ttl.into()]
}

#[tokio::test]
async fn a_refused_registration_arms_no_renewal() {
    let (bbmd, requests) = mock_bbmd(true).await;
    let client = client().await;
    let session = register(&client, &args(bbmd, "60")).await;
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert!(session.bbmd_registration.is_none());
}

#[tokio::test]
async fn an_accepted_registration_arms_renewal() {
    let (bbmd, requests) = mock_bbmd(false).await;
    let client = client().await;
    let mut session = register(&client, &args(bbmd, "60")).await;
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    let registration = session.bbmd_registration.as_ref().expect("renewal armed");
    assert_eq!(registration.ttl, 60);
    session.cancel_bbmd_renewal();
}

#[tokio::test]
async fn a_zero_ttl_is_refused_before_sending() {
    let (bbmd, requests) = mock_bbmd(false).await;
    let client = client().await;
    let session = register(&client, &args(bbmd, "0")).await;
    assert!(session.bbmd_registration.is_none());
    // A registration that is sent then lands as the mock's only request: a
    // stray one from the refused command would have come first.
    let mut accepted = register(&client, &args(bbmd, "60")).await;
    assert!(accepted.bbmd_registration.is_some());
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    accepted.cancel_bbmd_renewal();
}

#[test]
fn renewal_waits_at_least_a_second() {
    assert_eq!(renewal_interval(0), Duration::from_secs(1));
    assert_eq!(renewal_interval(1), Duration::from_secs(1));
    assert_eq!(renewal_interval(60), Duration::from_secs(48));
    assert_eq!(renewal_interval(u16::MAX), Duration::from_secs(52_428));
}
