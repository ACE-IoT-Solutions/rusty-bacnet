//! Configuration-time validation of BACnet/SC hub URIs.

use bacnet_types::error::Error;

use crate::sc::{ScConnectError, ScWebSocketErrorKind};

/// Validate a BACnet/SC hub URI without opening a connection.
///
/// The URI must use the `wss` scheme and include a nonempty host. This applies
/// the same parsing and policy as [`super::TlsWebSocket::connect`], so callers
/// can reject configuration before loading credentials or performing network
/// I/O.
pub fn validate_wss_uri(url: &str) -> Result<(), Error> {
    parse_wss_uri(url).map(|_| ())
}

pub(super) fn parse_wss_uri(url: &str) -> Result<tokio_tungstenite::tungstenite::http::Uri, Error> {
    let uri: tokio_tungstenite::tungstenite::http::Uri = url.parse().map_err(|e| {
        ScConnectError::WebSocket {
            http_status: None,
            untrusted_peer: None,
            tls: None,
            kind: ScWebSocketErrorKind::InvalidUrl,
            message: format!("Invalid WebSocket URL: {e}"),
        }
        .into_bacnet_error()
    })?;

    if !uri
        .scheme_str()
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("wss"))
    {
        return Err(ScConnectError::WebSocket {
            http_status: None,
            untrusted_peer: None,
            tls: None,
            kind: ScWebSocketErrorKind::UnsupportedScheme,
            message: "BACnet/SC WebSocket URI must use wss scheme".into(),
        }
        .into_bacnet_error());
    }

    if uri.host().map(str::is_empty).unwrap_or(true) {
        return Err(ScConnectError::WebSocket {
            http_status: None,
            untrusted_peer: None,
            tls: None,
            kind: ScWebSocketErrorKind::MissingHubHost,
            message: "BACnet/SC WebSocket URI must include a hub host".into(),
        }
        .into_bacnet_error());
    }

    Ok(uri)
}
