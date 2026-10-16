//! Configured concrete and erased clients expose the same owned observation.
#![cfg(feature = "sc-tls")]

use bacnet_client::client::BACnetClient;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::mstp::NoSerial;
use bacnet_transport::sc_hub::{ScHub, ScHubTlsConfig};
use bacnet_transport::sc_tls::ScNodeTlsConfig;
use tokio_rustls::rustls::pki_types::PrivatePkcs8KeyDer;

#[tokio::test]
async fn configured_concrete_and_erased_clients_observe_their_accepted_hub() {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let key = rcgen::KeyPair::generate().unwrap();
    let ca = params.self_signed(&key).unwrap().der().clone();
    let issuer = rcgen::Issuer::from_params(&params, &key);
    let leaf = |name: &str| {
        let mut params = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        let key = rcgen::KeyPair::generate().unwrap();
        (
            params.signed_by(&key, &issuer).unwrap().der().clone(),
            PrivatePkcs8KeyDer::from(key.serialize_der()),
        )
    };
    let (hub_der, hub_key) = leaf("client fixture hub");
    let (node_der, node_key) = leaf("client fixture node");
    let tls = ScNodeTlsConfig::from_der(vec![ca.clone()], vec![node_der], node_key.into()).unwrap();
    let config = ScHubTlsConfig::from_der(vec![ca], vec![hub_der.clone()], hub_key.into()).unwrap();
    let mut hub = ScHub::start("127.0.0.1:0", config, [0x10; 6], [0x10; 16])
        .await
        .unwrap();
    let url = format!("wss://localhost:{}", hub.local_addr().unwrap().port());
    let mut concrete = BACnetClient::sc_builder()
        .hub_url(&url)
        .tls_config(tls.clone())
        .vmac([0x22; 6])
        .device_uuid([0x22; 16])
        .build()
        .await
        .unwrap();
    let mut erased = BACnetClient::sc_builder()
        .hub_url(&url)
        .tls_config(tls)
        .vmac([0x33; 6])
        .device_uuid([0x33; 16])
        .build_any::<NoSerial>()
        .await
        .unwrap();
    let concrete_peer = concrete.transport().hub_peer_info().await.unwrap();
    let AnyTransport::Sc(sc) = erased.transport() else {
        panic!("expected SC");
    };
    let erased_peer = sc.hub_peer_info().await.unwrap();
    assert_eq!(
        concrete_peer.tls().certificates(),
        erased_peer.tls().certificates()
    );
    assert_eq!(
        concrete_peer.tls().leaf_sha256(),
        erased_peer.tls().leaf_sha256()
    );
    assert_eq!(
        concrete_peer.tls().peer_address(),
        hub.local_addr().unwrap()
    );
    assert_eq!(
        concrete_peer.tls().certificates()[0].der(),
        hub_der.as_ref()
    );
    assert_ne!(
        concrete_peer.tls().connection_id(),
        erased_peer.tls().connection_id()
    );
    concrete.stop().await.unwrap();
    erased.stop().await.unwrap();
    assert!(concrete.transport().hub_peer_info().await.is_none());
    drop(concrete);
    drop(erased);
    assert_eq!(
        concrete_peer.tls().certificates()[0]
            .metadata()
            .unwrap()
            .subject(),
        "CN=client fixture hub"
    );
    hub.stop().await;
}
