use super::*;
use crate::port_ownership::connect_loopback;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

#[tokio::test]
async fn disable_nagle_sets_tcp_nodelay_on_both_ends() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dialed = connect_loopback(listener.local_addr().unwrap()).await;
    let (accepted, _) = listener.accept().await.unwrap();
    for stream in [&dialed, &accepted] {
        assert!(!stream.nodelay().unwrap(), "Nagle is on by default");
        disable_nagle(stream);
        assert!(stream.nodelay().unwrap());
    }
}

#[test]
fn parse_wss_uri_accepts_secure_websocket_scheme() {
    assert!(parse_wss_uri("wss://hub.example.com:443").is_ok());
    assert!(validate_wss_uri("wss://hub.example.com:443").is_ok());
}

#[test]
fn parse_wss_uri_preserves_configured_hub_authority_path_and_query() {
    let uri = parse_wss_uri("wss://hub.example.com:47808/.bacnet/sc?profile=primary").unwrap();

    assert_eq!(uri.scheme_str(), Some("wss"));
    assert_eq!(uri.host(), Some("hub.example.com"));
    assert_eq!(uri.port_u16(), Some(47808));
    assert_eq!(
        uri.path_and_query().map(|path| path.as_str()),
        Some("/.bacnet/sc?profile=primary")
    );
}

#[test]
fn parse_wss_uri_rejects_plain_websocket_scheme() {
    let err = parse_wss_uri("ws://hub.example.com:80").unwrap_err();
    assert!(err.to_string().contains("wss"));
    assert_eq!(
        ScConnectError::from_error(&err),
        Some(&ScConnectError::WebSocket {
            http_status: None,
            kind: ScWebSocketErrorKind::UnsupportedScheme,
            message: "BACnet/SC WebSocket URI must use wss scheme".into(),
            untrusted_peer: None,
            tls: None,
            close_status: None,
        })
    );
}

#[test]
fn parse_wss_uri_rejects_non_websocket_scheme() {
    let err = parse_wss_uri("https://hub.example.com").unwrap_err();
    assert!(err.to_string().contains("wss"));
}

#[test]
fn parse_wss_uri_rejects_missing_hub_host() {
    let err = parse_wss_uri("wss://:443").unwrap_err();
    assert!(err.to_string().contains("hub host"));
}

#[test]
fn parse_wss_uri_rejects_malformed_hub_uri() {
    let err = parse_wss_uri("wss://[::1").unwrap_err();
    assert!(err.to_string().contains("Invalid WebSocket URL"));
}

#[test]
fn tls_server_name_from_uri_accepts_ipv6_literal() {
    let uri = parse_wss_uri("wss://[::1]:47808").unwrap();
    assert!(matches!(
        tls_server_name_from_uri(&uri).unwrap(),
        ServerName::IpAddress(_)
    ));
}

#[test]
fn verify_hub_subprotocol_accepts_selected_hub_protocol() {
    let response = tokio_tungstenite::tungstenite::http::Response::builder()
        .header("Sec-WebSocket-Protocol", BACNET_SC_HUB_SUBPROTOCOL)
        .body(None)
        .unwrap();

    assert!(verify_hub_subprotocol(&response).is_ok());
}

#[test]
fn verify_hub_subprotocol_rejects_missing_or_wrong_protocol() {
    let missing = tokio_tungstenite::tungstenite::http::Response::builder()
        .body(None)
        .unwrap();
    let err = verify_hub_subprotocol(&missing).unwrap_err();
    assert!(matches!(
        ScConnectError::from_error(&err),
        Some(ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::HubSubprotocolMissing,
            ..
        })
    ));

    let wrong = tokio_tungstenite::tungstenite::http::Response::builder()
        .header("Sec-WebSocket-Protocol", "dc.bsc.bacnet.org")
        .body(None)
        .unwrap();
    let err = verify_hub_subprotocol(&wrong).unwrap_err();
    assert!(matches!(
        ScConnectError::from_error(&err),
        Some(ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::HubSubprotocol,
            ..
        })
    ));
}

#[test]
fn websocket_handshake_error_maps_to_typed_variant() {
    // The connection ended before the answer was whole: no more
    // specific kind applies.
    let err = map_websocket_upgrade_error(
        TungsteniteError::Protocol(
            tokio_tungstenite::tungstenite::error::ProtocolError::HandshakeIncomplete,
        ),
        BACNET_SC_HUB_SUBPROTOCOL,
    );

    assert!(matches!(
        ScConnectError::from_error(&err),
        Some(ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::WebSocketHandshake,
            ..
        })
    ));
}

#[test]
fn websocket_upgrade_io_error_maps_to_websocket_handshake() {
    let err = map_websocket_upgrade_error(
        TungsteniteError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "closed during upgrade",
        )),
        BACNET_SC_HUB_SUBPROTOCOL,
    );

    assert!(matches!(
        ScConnectError::from_error(&err),
        Some(ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::WebSocketHandshake,
            ..
        })
    ));
}

fn test_tls_config() -> ScNodeTlsConfig {
    test_tls_pair().0
}

pub(super) fn test_tls_pair() -> (ScNodeTlsConfig, Arc<tokio_rustls::rustls::ServerConfig>) {
    let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();

    let mut ca_params =
        rcgen::CertificateParams::new(Vec::<String>::new()).expect("empty SANs are valid");
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let ca_issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);

    let server_params =
        rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
    let server_key = rcgen::KeyPair::generate().unwrap();
    let server_cert = server_params.signed_by(&server_key, &ca_issuer).unwrap();

    let server_chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(server_cert.pem().as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    let server_key = PrivateKeyDer::from_pem_slice(server_key.serialize_pem().as_bytes()).unwrap();
    let server_config = tokio_rustls::rustls::ServerConfig::builder_with_protocol_versions(&[
        &tokio_rustls::rustls::version::TLS13,
    ])
    .with_no_client_auth()
    .with_single_cert(server_chain, server_key)
    .unwrap();

    let ca_certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(ca_cert.pem().as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    let client_key = rcgen::KeyPair::generate().unwrap();
    let client_cert = rcgen::CertificateParams::new(vec!["node".into()])
        .unwrap()
        .signed_by(&client_key, &ca_issuer)
        .unwrap();
    let client_config = ScNodeTlsConfig::from_der(
        ca_certs,
        vec![client_cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(client_key.serialize_der()).into(),
    )
    .unwrap();

    (client_config, Arc::new(server_config))
}

/// A loopback address nothing listens on, for a refused dial. A port
/// only bound refuses nothing on macOS (the SYN goes unanswered), so the
/// listener is dropped; until it is dialled another process could take
/// the port, and Windows can fail a loopback connect on a local port
/// collision. Either way the dial is not refused, so callers try again.
pub(crate) fn closed_port() -> std::net::SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

#[tokio::test]
async fn tls_websocket_connect_dial_failure_is_typed() {
    let mut last = None;
    for _ in 0..5 {
        let url = format!("wss://{}", closed_port());
        // Bounded: a process that took the port could accept and stay
        // silent; that attempt doesn't count.
        let dial = TlsWebSocket::connect(&url, test_tls_config());
        let err = match tokio::time::timeout(std::time::Duration::from_secs(10), dial).await {
            Ok(Err(err)) => err,
            Ok(Ok(_)) | Err(_) => continue,
        };
        // Nothing listens: the peer refused it, which has its own code.
        if matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::TcpRefused,
                ..
            })
        ) {
            assert_eq!(
                crate::sc::ScDisconnectCause::from_error(&err).error_code(),
                Some(bacnet_types::enums::ErrorCode::TCP_CONNECTION_REFUSED)
            );
            return;
        }
        last = Some(err);
    }
    panic!("no dial was refused; the last error: {last:?}");
}

/// A name that does not resolve is a host lookup failure, not a TCP one.
#[tokio::test]
async fn tls_websocket_connect_lookup_failure_is_typed() {
    // `.invalid` never resolves (RFC 6761).
    let err = match TlsWebSocket::connect("wss://hub.invalid:443", test_tls_config()).await {
        Ok(_) => panic!("expected a host lookup failure"),
        Err(err) => err,
    };
    assert!(matches!(
        ScConnectError::from_error(&err),
        Some(ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::HostLookup,
            tls: None,
            ..
        })
    ));
}

#[tokio::test]
async fn tls_websocket_connect_tls_failure_is_typed() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accept_task = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
    });

    let err = match TlsWebSocket::connect(&format!("wss://{addr}"), test_tls_config()).await {
        Ok(_) => panic!("expected TLS handshake failure"),
        Err(err) => err,
    };

    // The peer closed mid-handshake: the TLS library gave no reason, so
    // none is attached and the cause stays the plain handshake kind.
    assert!(matches!(
        ScConnectError::from_error(&err),
        Some(ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::TlsHandshake,
            tls: None,
            ..
        })
    ));
    assert_eq!(
        crate::sc::ScDisconnectCause::from_error(&err),
        crate::sc::ScDisconnectCause::WebSocket(ScWebSocketErrorKind::TlsHandshake)
    );
    accept_task.await.unwrap();
}

#[tokio::test]
async fn tls_websocket_connect_upgrade_io_failure_is_websocket_handshake() {
    let (client_config, server_config) = test_tls_pair();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accept_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut tls = TlsAcceptor::from(server_config)
            .accept(socket)
            .await
            .unwrap();
        let mut buf = [0u8; 128];
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), tls.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        tls.shutdown().await.unwrap();
    });

    let err = match TlsWebSocket::connect(&format!("wss://{addr}"), client_config).await {
        Ok(_) => panic!("expected WebSocket upgrade failure"),
        Err(err) => err,
    };

    assert!(matches!(
        ScConnectError::from_error(&err),
        Some(ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::WebSocketHandshake,
            ..
        })
    ));
    accept_task.await.unwrap();
}
