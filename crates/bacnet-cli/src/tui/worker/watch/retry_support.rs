//! A request-level peer for typed failure and managed-handle lifecycle tests.

use std::sync::atomic::{AtomicUsize, Ordering};

use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::MacAddr;
use bytes::Bytes;

use super::*;

#[derive(Clone, Copy)]
pub(super) enum Reply {
    Silent,
    Transport,
    Abort(u8),
    Reject(u8),
    Protocol,
    SendProtocol,
    Ack,
}

struct Peer {
    inbox: mpsc::Sender<ReceivedNpdu>,
    receiver: Option<mpsc::Receiver<ReceivedNpdu>>,
    reply: Reply,
    requests: Arc<AtomicUsize>,
}

impl TransportPort for Peer {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.receiver.take().unwrap())
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let decoded = decode_npdu(Bytes::copy_from_slice(npdu))?;
        let apdu = &decoded.payload;
        if decoded.is_network_message() || apdu.len() < 4 || apdu[0] >> 4 != 0 {
            return Ok(());
        }
        self.requests.fetch_add(1, Ordering::Relaxed);
        let invoke = apdu[2];
        let service = apdu[3];
        let reply = match self.reply {
            Reply::Silent => return Ok(()),
            Reply::SendProtocol => {
                return Err(Error::protocol(
                    bacnet_types::enums::ErrorClass::SERVICES,
                    bacnet_types::enums::ErrorCode::OTHER,
                ));
            }
            Reply::Transport => {
                return Err(Error::Transport(std::io::Error::other("test send failed")));
            }
            Reply::Abort(reason) => vec![0x71, invoke, reason],
            Reply::Reject(reason) => vec![0x60, invoke, reason],
            Reply::Protocol => vec![0x50, invoke, service, 0x91, 5, 0x91, 9],
            Reply::Ack => vec![0x20, invoke, service],
        };
        let mut frame = vec![1, 0];
        frame.extend(reply);
        self.inbox
            .try_send(ReceivedNpdu::unverified(
                Bytes::from(frame),
                MacAddr::from_slice(mac),
                false,
                Vec::new(),
                None,
            ))
            .unwrap();
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &[1]
    }
}

pub(super) async fn client(
    reply: Reply,
) -> (Arc<BACnetClient<impl TransportPort>>, Arc<AtomicUsize>) {
    let (inbox, receiver) = mpsc::channel(16);
    let requests = Arc::new(AtomicUsize::new(0));
    let peer = Peer {
        inbox,
        receiver: Some(receiver),
        reply,
        requests: requests.clone(),
    };
    let client = BACnetClient::generic_builder()
        .transport(peer)
        .apdu_timeout_ms(2_000)
        .build()
        .await
        .unwrap();
    (Arc::new(client), requests)
}

pub(super) async fn stop<T: TransportPort + 'static>(client: Arc<BACnetClient<T>>) {
    let mut client = Arc::try_unwrap(client).unwrap_or_else(|_| panic!("client still retained"));
    client.stop().await.unwrap();
}
