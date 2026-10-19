//! The discovery worker (#960): sends Who-Has and Who-Is-Router-To-Network
//! when asked, and passes on every I-Have and network report the client
//! hears, asked for or not.
//!
//! It runs beside the session loop under `join!`. A send is one datagram
//! and holds no lock. Answers are not collected here: the client's
//! broadcast channels stream them, and the UI keeps what matches its
//! search. A receiver that falls behind loses the oldest, which are counted
//! as dropped events, like a full UI channel.

use bacnet_client::client::{BACnetClient, ReceivedIHave};
use bacnet_network::layer::{NetworkReport, ReceivedNetworkReport};
use bacnet_services::who_has::WhoHasObject;
use bacnet_transport::port::TransportPort;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use super::EventSink;
use crate::core::target::AddressStyle;
use crate::tui::message::WorkerEvent;
use crate::tui::network::discovery::{DiscoveryCommand, DiscoveryEvent, FoundRow};

/// What an I-Have reports, as the UI shows it.
fn found(reply: &ReceivedIHave, style: AddressStyle) -> FoundRow {
    FoundRow {
        device: reply.i_have.device_identifier.instance_number(),
        object: reply.i_have.object_identifier,
        name: reply.i_have.object_name.clone(),
        address: style.format(&reply.source_mac),
        network: reply.source_network.as_ref().map(|source| source.network),
    }
}

/// What a network report says, as the UI shows it.
fn report(received: ReceivedNetworkReport, style: AddressStyle) -> Option<DiscoveryEvent> {
    let from = style.format(&received.source_mac);
    Some(match received.report {
        NetworkReport::IAmRouterToNetwork(networks) => DiscoveryEvent::Router {
            router: from,
            networks,
        },
        NetworkReport::NetworkNumberIs {
            network,
            configured,
        } => DiscoveryEvent::NetworkNumber {
            network,
            configured,
        },
        NetworkReport::RejectMessageToNetwork(reject) => DiscoveryEvent::Rejected {
            router: from,
            network: reject.network,
            reason: reject.reason.to_string(),
        },
        _ => return None,
    })
}

/// Serve Who-Has and router discovery until the session ends.
pub(crate) async fn run<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    sink: &EventSink,
    mut commands: mpsc::Receiver<DiscoveryCommand>,
    style: AddressStyle,
) {
    let mut i_haves = client.i_have_events();
    let mut reports = client.network_reports();
    loop {
        tokio::select! {
            biased;
            () = sink.cancelled() => break,
            command = commands.recv() => {
                let Some(command) = command else { break };
                let sent = match &command {
                    DiscoveryCommand::WhoHas { name } => {
                        client.who_has(WhoHasObject::Name(name.clone()), None).await
                    }
                    DiscoveryCommand::WhoIsRouter => client.who_is_router_to_network(None, None).await,
                };
                if let Err(error) = sent {
                    let reason = error.to_string();
                    let event = DiscoveryEvent::Refused { command, reason };
                    sink.deliver(WorkerEvent::Discovery(event)).await;
                }
            }
            received = i_haves.recv() => match received {
                Ok(reply) => {
                    sink.offer(WorkerEvent::Discovery(DiscoveryEvent::IHave(found(&reply, style))));
                }
                Err(RecvError::Lagged(missed)) => sink.add_dropped(missed),
                Err(RecvError::Closed) => break,
            },
            received = reports.recv() => match received {
                Ok(received) => {
                    if let Some(event) = report(received, style) {
                        sink.offer(WorkerEvent::Discovery(event));
                    }
                }
                Err(RecvError::Lagged(missed)) => sink.add_dropped(missed),
                Err(RecvError::Closed) => break,
            },
        }
    }
}
