//! Local BACnet/SC receive and WebSocket resource budgets.

/// Node receive capacity is independent of every negotiated egress path.
pub(crate) const LOCAL_RECEIVE_APDU_CAPACITY: u16 = 1476;
pub(crate) const LOCAL_NPDU_HEADER_LEN: u16 = 2;
/// Table 6-1 minimum routed NPDU length for BACnet/SC.
pub const MIN_ROUTED_NPDU_LENGTH: u16 = 1497;
/// Table 6-1 maximum BACnet/SC NPDU after reserving required Data Options.
pub const MAX_NPDU_LENGTH: u16 = 61_327;
/// Minimum Data Options content a secure-connect router forwards between SC ports.
pub const MIN_ROUTED_DATA_OPTIONS_LENGTH: u16 = 4_192;
/// Fixed BVLC header length used by the Table 6-1 maximum calculation.
pub const BVLC_HEADER_LENGTH: u16 = 16;
pub(crate) const LOCAL_RECEIVE_NPDU_CAPACITY: u16 = MIN_ROUTED_NPDU_LENGTH;

// Annex AB.5.1 hub workload: 16-byte envelope + 4192 encoded option bytes
// + 1497-byte NPDU. Nodes use this full-message default independently of NPDU.
pub(crate) const DEFAULT_MAX_BVLC_LENGTH: u16 = 5705;

pub(crate) fn validate_router_receive_limits(
    max_npdu_length: u16,
    max_bvlc_length: u16,
) -> Result<(), bacnet_types::error::Error> {
    if !(MIN_ROUTED_NPDU_LENGTH..=MAX_NPDU_LENGTH).contains(&max_npdu_length) {
        return Err(bacnet_types::error::Error::Encoding(format!(
            "SC router max NPDU length must be in {MIN_ROUTED_NPDU_LENGTH}..={MAX_NPDU_LENGTH}"
        )));
    }
    let required_bvlc =
        max_npdu_length as u32 + MIN_ROUTED_DATA_OPTIONS_LENGTH as u32 + BVLC_HEADER_LENGTH as u32;
    if (max_bvlc_length as u32) < required_bvlc {
        return Err(bacnet_types::error::Error::Encoding(format!(
            "SC router max BVLC length must be at least {required_bvlc} for max NPDU length {max_npdu_length}"
        )));
    }
    Ok(())
}

#[cfg(feature = "sc-tls")]
pub(crate) fn websocket(
    max_receive: usize,
) -> tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
    // Outbound peer capacity is independent of our receive limit. Permit two
    // full u16-sized messages plus worst-case framing, with a bounded queue.
    tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .read_buffer_size(4096)
        .write_buffer_size(4096)
        .max_write_buffer_size(2 * (u16::MAX as usize + 14))
        .max_frame_size(Some(max_receive))
        .max_message_size(Some(max_receive))
}

#[cfg(all(test, feature = "sc-tls"))]
mod tests {
    use super::*;
    use std::io::{self, Read, Write};
    use tokio_tungstenite::tungstenite::{protocol::Role, Error, Message, WebSocket};

    struct Blocked;
    impl Read for Blocked {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }
    impl Write for Blocked {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }

    #[test]
    fn websocket_write_queue_is_bounded_for_blocked_io_and_full_peer_capacity() {
        for (role, receive_limit) in [(Role::Server, 5705), (Role::Client, 65535)] {
            let config = websocket(receive_limit);
            assert_eq!(
                (config.read_buffer_size, config.write_buffer_size),
                (4096, 4096)
            );
            assert_eq!(config.max_write_buffer_size, 131098);
            assert!(!config.accept_unmasked_frames);
            let mut socket = WebSocket::from_raw_socket(Blocked, role, Some(config));
            let message = Message::Binary(vec![0x55; 65535].into());
            for _ in 0..2 {
                assert!(
                    matches!(socket.write(message.clone()), Err(Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock)
                );
            }
            assert!(matches!(
                socket.write(message),
                Err(Error::WriteBufferFull(_))
            ));
        }
    }
}
