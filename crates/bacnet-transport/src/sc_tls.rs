//! TLS WebSocket implementation for BACnet/SC.
//!
//! Provides [`TlsWebSocket`], a [`WebSocketPort`] backed by `tokio-tungstenite`
//! with `rustls` TLS.  This is the production WebSocket driver used by
//! [`crate::sc::ScTransport`] when connecting to a real BACnet/SC hub.

mod config_error;
pub use config_error::{ScCertificateForm, ScKeyForm, ScPemFault, ScTlsConfigError, ScTlsInput};
pub mod inspection;
pub(crate) mod material;
pub use material::MAX_PEM_FILE_BYTES;
mod peer_info;
pub use peer_info::{ScCertificateMetadata, ScHandshakeKind, ScPeerCertificate, ScTlsPeerInfo};
mod subject_alt_name;
pub use subject_alt_name::ScSubjectAltName;
mod attempt_capture;
mod tls_failure;
mod untrusted_peer;
mod websocket_failure;
pub use untrusted_peer::ScUntrustedPeerInfo;
use websocket_failure::{
    closed_by_peer, ended_without_close, map_websocket_upgrade_error, tcp_dial_kind,
    verify_direct_subprotocol, verify_hub_subprotocol,
};
mod tls_config;
pub use tls_config::ScNodeTlsConfig;

pub(crate) mod direct_accept;
pub use direct_accept::{
    DIRECT_ACCEPT_IDLE_TIMEOUT, DIRECT_ACCEPT_MAX_ESTABLISHED_PEERS, DirectAcceptConfig,
    DirectListener,
};

use futures_util::{SinkExt, StreamExt};
use tokio::{net::TcpStream, sync::Mutex};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_tungstenite::WebSocketStream;
#[cfg(test)]
use tokio_tungstenite::tungstenite::Error as TungsteniteError;
use tokio_tungstenite::tungstenite::Message;

use bacnet_types::error::Error;

use crate::sc::{ScConnectError, ScDialStage, ScWebSocketErrorKind, WebSocketPort};
use crate::sc_frame::{BACNET_SC_DIRECT_SUBPROTOCOL, BACNET_SC_HUB_SUBPROTOCOL};

type WsStream = WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>>;

/// Turn off Nagle's algorithm on an SC TCP stream before TLS. SC traffic is
/// small WebSocket messages; with Nagle on, a message written before the
/// previous one is acknowledged waits for that ACK, which a peer with nothing
/// to send back can delay by tens to hundreds of milliseconds, depending on the
/// peer's OS (#900). A failure only costs latency, so it is logged and the
/// connection proceeds.
pub(crate) fn disable_nagle(stream: &TcpStream) {
    if let Err(error) = stream.set_nodelay(true) {
        let peer = stream.peer_addr().ok();
        tracing::debug!(%error, ?peer, "SC TCP stream keeps Nagle's algorithm");
    }
}

/// A TLS-secured WebSocket connection implementing [`WebSocketPort`].
///
/// Created via [`TlsWebSocket::connect`], which performs the TLS handshake and
/// WebSocket upgrade in one step.
pub struct TlsWebSocket {
    pub(crate) verified_leaf: [u8; 32],
    peer_info: std::sync::Arc<ScTlsPeerInfo>,
    pub(crate) peer_address: std::net::SocketAddr,
    write: Mutex<futures_util::stream::SplitSink<WsStream, Message>>,
    read: Mutex<futures_util::stream::SplitStream<WsStream>>,
}

impl TlsWebSocket {
    /// Immutable certificate information from this physical TLS/WebSocket connection.
    /// This alone does not establish BACnet/SC admission. No I/O is performed.
    pub fn peer_info(&self) -> &std::sync::Arc<ScTlsPeerInfo> {
        &self.peer_info
    }

    // One physical WebSocket frame per worker turn, including controls. The
    // worker owns application/control writes; this adapter never takes write.
    pub(crate) async fn direct_frame(
        &self,
    ) -> Result<crate::sc::direct_socket::DirectFrame, Error> {
        direct_frame(&mut *self.read.lock().await).await
    }

    /// Connect to a WebSocket endpoint with TLS.
    ///
    /// `url` must be a `wss://` URL. The validated local policy supplies explicit
    /// trust and operational credentials; see [`ScNodeTlsConfig`] for the
    /// certificate-request and normal resumption limits.
    ///
    /// ```
    /// use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
    /// async fn connect(url: &str, tls: ScNodeTlsConfig)
    ///     -> Result<TlsWebSocket, bacnet_types::error::Error> {
    ///     TlsWebSocket::connect(url, tls).await
    /// }
    /// ```
    ///
    /// ```compile_fail,E0308
    /// use bacnet_transport::sc_tls::TlsWebSocket;
    /// async fn raw(config: std::sync::Arc<rustls::ClientConfig>) {
    ///     let _ = TlsWebSocket::connect("wss://localhost", config).await;
    /// }
    /// ```
    pub async fn connect(url: &str, tls_config: ScNodeTlsConfig) -> Result<Self, Error> {
        boxed(|| Self::connect_with_subprotocol(url, tls_config, BACNET_SC_HUB_SUBPROTOCOL)).await
    }

    /// Check a `wss` URL the way [`Self::connect`] parses it (scheme, host,
    /// TLS server name), without any I/O.
    pub fn validate_url(url: &str) -> Result<(), Error> {
        let uri = parse_wss_uri(url)?;
        tcp_addr_from_uri(&uri)?;
        tls_server_name_from_uri(&uri).map(drop)
    }

    /// Dial a direct-connection peer `wss` URI with node operational credentials.
    ///
    /// This is the dial-out half of an optional direct connection: the caller
    /// supplies a peer URI (statically configured or previously discovered),
    /// and this performs the TCP dial, mutual-TLS handshake, and WebSocket
    /// upgrade offering only the direct subprotocol. Peer validation reuses
    /// the [`ScNodeTlsConfig`] policy — explicit CA trust, operational
    /// certificate checks during TLS, TLS 1.3 only, and URI-host server-name
    /// binding — exactly as for hub dial. No BACnet identity (VMAC/UUID)
    /// binding is performed here; that belongs to the later connection
    /// exchange. Discovery triggering, routing over the connection, inbound
    /// (accept-side) paths, and hub/failover interaction are out of scope.
    pub async fn connect_direct(url: &str, tls_config: ScNodeTlsConfig) -> Result<Self, Error> {
        boxed(|| Self::connect_with_subprotocol(url, tls_config, BACNET_SC_DIRECT_SUBPROTOCOL))
            .await
    }

    // Each stage's future is boxed: the tokio-tungstenite handshake below
    // already takes about 125 KB of stack in a debug build, and unboxed, these
    // futures made this frame and connect's 51 KB rather than 24 KB (#953).
    async fn connect_with_subprotocol(
        url: &str,
        tls_config: ScNodeTlsConfig,
        subprotocol: &'static str,
    ) -> Result<Self, Error> {
        let uri = parse_wss_uri(url)?;
        let addr = tcp_addr_from_uri(&uri)?;
        let server_name = tls_server_name_from_uri(&uri)?;
        let intended_server_name = server_name.to_str().into_owned();
        let request = tokio_tungstenite::tungstenite::ClientRequestBuilder::new(uri)
            .with_sub_protocol(subprotocol);

        crate::sc::enter_dial_stage(ScDialStage::HostLookup);
        let resolved = boxed(|| crate::tcp_connect::resolve(&addr))
            .await
            .map_err(|e| {
                ScConnectError::websocket(
                    ScWebSocketErrorKind::HostLookup,
                    format!("WebSocket host lookup for {addr} failed: {e}"),
                )
                .into_bacnet_error_with_io_kind(e.kind())
            })?;
        crate::sc::enter_dial_stage(ScDialStage::TcpConnect);
        let socket = boxed(|| crate::tcp_connect::connect_resolved(&resolved))
            .await
            .map_err(|e| {
                // A refusal and no route have their own codes (Annex
                // AB.7.5.1). With several addresses, a refusal on any one
                // reports the dial refused, even when another address timed
                // out or had no route.
                ScConnectError::websocket(
                    tcp_dial_kind(e.kind()),
                    format!("WebSocket TCP dial to {addr} failed: {e}"),
                )
                .into_bacnet_error_with_io_kind(e.kind())
            })?;

        disable_nagle(&socket);
        let peer_address = socket.peer_addr().map_err(Error::Transport)?;
        crate::sc::enter_dial_stage(ScDialStage::Tls);
        // Both constructing and polling Rustls's handshake future happen in
        // this attempt's observation scope. The shared verifier stays stable.
        let (result, observed) = attempt_capture::capture(async move {
            boxed(|| tls_config.into_connector().connect(server_name, socket)).await
        })
        .await;
        let tls_stream = result.map_err(|e| {
            ScConnectError::WebSocket {
                http_status: None,
                close_status: None,
                untrusted_peer: observed.map(|chain| {
                    std::sync::Arc::new(ScUntrustedPeerInfo::new(
                        peer_address,
                        intended_server_name.clone(),
                        chain,
                    ))
                }),
                tls: tls_failure::classify(&e),
                kind: ScWebSocketErrorKind::TlsHandshake,
                message: format!("WebSocket TLS handshake with {addr} failed: {e}"),
            }
            .into_bacnet_error()
        })?;

        let verified_leaf =
            direct_accept::verified_leaf_sha256(tls_stream.get_ref().1.peer_certificates())
                .ok_or_else(|| tls_failure::no_peer_certificate(&addr))?;
        let peer_info = std::sync::Arc::new(ScTlsPeerInfo::capture(
            tls_stream.get_ref().1,
            peer_address,
            intended_server_name,
            verified_leaf,
        ));
        crate::sc::enter_dial_stage(ScDialStage::WebSocketUpgrade);
        let (ws_stream, response) = boxed(|| {
            tokio_tungstenite::client_async_with_config(
                request,
                tls_stream,
                // Public local capacities remain mutable u16 values. The adapter
                // bounds its first Vec copy; protocol layers apply current limits.
                Some(crate::sc_limits::websocket(u16::MAX as usize)),
            )
        })
        .await
        .map_err(|error| map_websocket_upgrade_error(error, subprotocol))?;
        if subprotocol == BACNET_SC_DIRECT_SUBPROTOCOL {
            verify_direct_subprotocol(&response)?;
        } else {
            verify_hub_subprotocol(&response)?;
        }

        let (write, read) = ws_stream.split();
        Ok(Self {
            verified_leaf,
            peer_info,
            peer_address,
            write: Mutex::new(write),
            read: Mutex::new(read),
        })
    }
}

/// Create the future `make` returns and move it to the heap, so awaiting it
/// costs the caller's state and debug-build poll frame a pointer rather than
/// the whole future (#953). For connection setup and per-connection tasks,
/// not per-frame paths.
pub(crate) fn boxed<F: std::future::Future>(make: impl FnOnce() -> F) -> std::pin::Pin<Box<F>> {
    Box::pin(make())
}

// A finite unit of receive scheduling, shared with deterministic frame tests.
async fn direct_frame<S, E>(read: &mut S) -> Result<crate::sc::direct_socket::DirectFrame, Error>
where
    S: futures_util::Stream<Item = Result<Message, E>> + Unpin,
    E: std::fmt::Display,
{
    use crate::sc::direct_socket::DirectFrame;
    match read.next().await {
        Some(Ok(Message::Binary(data))) => Ok(DirectFrame::Binary(data.to_vec())),
        Some(Ok(Message::Ping(_) | Message::Pong(_))) => Ok(DirectFrame::Control),
        // The caller answers it with a 1003 close (Annex AB.7.5.3, #2520).
        Some(Ok(Message::Text(_) | Message::Frame(_))) => Ok(DirectFrame::NonBinary),
        Some(Err(error)) => Err(Error::Transport(std::io::Error::other(error.to_string()))),
        _ => Err(Error::Transport(std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "direct WebSocket ended",
        ))),
    }
}

/// Validate a secure hub URL using the same rules as the transport's dialer.
pub fn validate_wss_uri(url: &str) -> Result<(), Error> {
    parse_wss_uri(url).map(|_| ())
}

fn parse_wss_uri(url: &str) -> Result<tokio_tungstenite::tungstenite::http::Uri, Error> {
    let uri: tokio_tungstenite::tungstenite::http::Uri = url.parse().map_err(|e| {
        ScConnectError::websocket(
            ScWebSocketErrorKind::InvalidUrl,
            format!("Invalid WebSocket URL: {e}"),
        )
        .into_bacnet_error()
    })?;

    if !uri
        .scheme_str()
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("wss"))
    {
        return Err(ScConnectError::websocket(
            ScWebSocketErrorKind::UnsupportedScheme,
            "BACnet/SC WebSocket URI must use wss scheme",
        )
        .into_bacnet_error());
    }

    if uri.host().map(str::is_empty).unwrap_or(true) {
        return Err(ScConnectError::websocket(
            ScWebSocketErrorKind::MissingHubHost,
            "BACnet/SC WebSocket URI must include a hub host",
        )
        .into_bacnet_error());
    }

    Ok(uri)
}

fn tcp_addr_from_uri(uri: &tokio_tungstenite::tungstenite::http::Uri) -> Result<String, Error> {
    let host = uri.host().filter(|host| !host.is_empty()).ok_or_else(|| {
        ScConnectError::websocket(
            ScWebSocketErrorKind::MissingHubHost,
            "BACnet/SC WebSocket URI must include a hub host",
        )
        .into_bacnet_error()
    })?;
    let port = uri.port_u16().unwrap_or(443);
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    Ok(format!("{host}:{port}"))
}

fn tls_server_name_from_uri(
    uri: &tokio_tungstenite::tungstenite::http::Uri,
) -> Result<ServerName<'static>, Error> {
    let host = uri.host().filter(|host| !host.is_empty()).ok_or_else(|| {
        ScConnectError::websocket(
            ScWebSocketErrorKind::MissingHubHost,
            "BACnet/SC WebSocket URI must include a hub host",
        )
        .into_bacnet_error()
    })?;

    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);

    ServerName::try_from(host.to_owned()).map_err(|e| {
        ScConnectError::websocket(
            ScWebSocketErrorKind::InvalidUrl,
            format!("Invalid WebSocket TLS server name: {e}"),
        )
        .into_bacnet_error()
    })
}

impl WebSocketPort for TlsWebSocket {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        let mut write = self.write.lock().await;
        write
            .send(Message::Binary(data.to_vec().into()))
            .await
            .map_err(|e| {
                ScConnectError::websocket(
                    ScWebSocketErrorKind::Send,
                    format!("WebSocket send failed: {e}"),
                )
                .into_bacnet_error()
            })
    }

    async fn recv(&self) -> Result<Vec<u8>, Error> {
        loop {
            // Read one message under the read lock, then drop it before
            // acquiring write (avoids read→write lock ordering deadlock).
            let msg = {
                let mut read = self.read.lock().await;
                read.next().await
            };
            match msg {
                Some(Ok(Message::Binary(data))) => return Ok(data.to_vec()),
                Some(Ok(Message::Close(frame))) => return Err(closed_by_peer(frame)),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                    continue;
                }
                Some(Ok(_)) => {
                    // Non-binary data frames: close with 1003, within the
                    // close grace as every close this node sends (#2520).
                    let close = Message::Close(Some(
                        tokio_tungstenite::tungstenite::protocol::CloseFrame {
                            code: crate::sc::CLOSE_DATA_NOT_ACCEPTED.into(),
                            reason: "BACnet/SC requires binary frames".into(),
                        },
                    ));
                    let send = async { self.write.lock().await.send(close).await };
                    let _ = tokio::time::timeout(crate::sc::CLOSE_GRACE, send).await;
                    return Err(ScConnectError::websocket(
                        ScWebSocketErrorKind::NonBinaryFrame,
                        "non-binary WebSocket frame received",
                    )
                    .into_bacnet_error());
                }
                Some(Err(e)) => {
                    let kind = if ended_without_close(&e) {
                        ScWebSocketErrorKind::StreamEnded
                    } else {
                        ScWebSocketErrorKind::Receive
                    };
                    return Err(ScConnectError::websocket(
                        kind,
                        format!("WebSocket recv error: {e}"),
                    )
                    .into_bacnet_error());
                }
                None => {
                    return Err(ScConnectError::websocket(
                        ScWebSocketErrorKind::StreamEnded,
                        "WebSocket stream ended",
                    )
                    .into_bacnet_error());
                }
            }
        }
    }

    fn awaits_disconnect_ack(&self) -> bool {
        true
    }

    /// Write the close frame and return; the transport drops the socket next,
    /// so the peer's answering close is never awaited (#2520).
    async fn close(&self, status: Option<u16>) -> Result<(), Error> {
        let frame = status.map(
            |code| tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: code.into(),
                reason: "".into(),
            },
        );
        let mut write = self.write.lock().await;
        write.send(Message::Close(frame)).await.map_err(|e| {
            ScConnectError::websocket(
                ScWebSocketErrorKind::Send,
                format!("WebSocket close failed: {e}"),
            )
            .into_bacnet_error()
        })
    }
}

#[cfg(test)]
#[path = "sc_tls/node_tls_tests.rs"]
mod node_tls_tests;

#[cfg(test)]
#[path = "sc_tls/direct_dial_tests.rs"]
mod direct_dial_tests;

#[cfg(test)]
mod tests;
