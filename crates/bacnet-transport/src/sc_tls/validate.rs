//! Configuration-time validation of BACnet/SC hub URIs.

use bacnet_types::error::Error;

use super::parse_wss_uri;

/// Validate a BACnet/SC hub URI without opening a connection.
///
/// The URI must use the `wss` scheme and include a nonempty host. This applies
/// the same parsing and policy as [`super::TlsWebSocket::connect`], so callers
/// can reject configuration before loading credentials or performing network
/// I/O.
pub fn validate_wss_uri(url: &str) -> Result<(), Error> {
    parse_wss_uri(url).map(|_| ())
}
