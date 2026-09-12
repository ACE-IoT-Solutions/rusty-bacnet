use super::*;
use bacnet_transport::port::{ReceivedNpdu, TransportHealthState};

struct HealthTestTransport {
    receive: Option<mpsc::Receiver<ReceivedNpdu>>,
    health: watch::Receiver<TransportHealth>,
    topology: String,
    mac: [u8; 1],
    start_error: Option<std::io::ErrorKind>,
}

impl HealthTestTransport {
    fn new(
        topology: impl Into<String>,
        mac: u8,
        health: TransportHealth,
    ) -> (Self, watch::Sender<TransportHealth>) {
        let (_tx, receive) = mpsc::channel(1);
        let (health_tx, health_rx) = watch::channel(health);
        (
            Self {
                receive: Some(receive),
                health: health_rx,
                topology: topology.into(),
                mac: [mac],
                start_error: None,
            },
            health_tx,
        )
    }

    fn failing(topology: impl Into<String>, mac: u8, error_kind: std::io::ErrorKind) -> Self {
        let (mut transport, _health) = Self::new(
            topology,
            mac,
            health(TransportHealthState::Down, "not started"),
        );
        transport.start_error = Some(error_kind);
        transport
    }
}

impl TransportPort for HealthTestTransport {
    fn transport_kind(&self) -> &'static str {
        "health-test"
    }

    fn topology_id(&self) -> Option<String> {
        Some(self.topology.clone())
    }

    fn health(&self) -> TransportHealth {
        self.health.borrow().clone()
    }

    fn health_changes(&self) -> Option<watch::Receiver<TransportHealth>> {
        Some(self.health.clone())
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        if let Some(kind) = self.start_error {
            return Err(Error::Transport(std::io::Error::new(
                kind,
                "simulated handshake failure",
            )));
        }
        self.receive
            .take()
            .ok_or_else(|| Error::Encoding("test transport already started".to_string()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.mac
    }
}

fn health(state: TransportHealthState, detail: &str) -> TransportHealth {
    TransportHealth {
        state,
        detail: Some(detail.to_string()),
        ..TransportHealth::default()
    }
}

#[tokio::test]
async fn duplicate_transport_topology_is_rejected_before_start() {
    let (first, _first_health) =
        HealthTestTransport::new("shared-topology", 1, health(TransportHealthState::Up, "up"));
    let (second, _second_health) =
        HealthTestTransport::new("shared-topology", 2, health(TransportHealthState::Up, "up"));

    let result = BACnetRouter::start(vec![
        RouterPort {
            transport: first,
            network_number: 100,
        },
        RouterPort {
            transport: second,
            network_number: 200,
        },
    ])
    .await;

    let error = result.err().expect("duplicate topology must be rejected");
    assert!(error.to_string().contains("Duplicate health-test topology"));
}

#[tokio::test]
async fn start_failure_identifies_configured_port_kind_and_identity() {
    let (first, _first_health) = HealthTestTransport::new(
        "healthy-topology",
        1,
        health(TransportHealthState::Up, "up"),
    );
    let second = HealthTestTransport::failing(
        "failed-hub-topology",
        2,
        std::io::ErrorKind::ConnectionRefused,
    );

    let error = BACnetRouter::start(vec![
        RouterPort {
            transport: first,
            network_number: 100,
        },
        RouterPort {
            transport: second,
            network_number: 200,
        },
    ])
    .await
    .err()
    .expect("second port must fail startup");

    let Error::Transport(source) = error else {
        panic!("transport error category was not preserved");
    };
    assert_eq!(source.kind(), std::io::ErrorKind::ConnectionRefused);
    let message = source.to_string();
    assert!(message.contains("router port 1"));
    assert!(message.contains("health-test"));
    assert!(message.contains("failed-hub-topology"));
    assert!(message.contains("simulated handshake failure"));
}

#[tokio::test]
async fn port_health_preserves_configuration_order_and_tracks_changes() {
    let (first, first_health) = HealthTestTransport::new(
        "health-a",
        1,
        health(TransportHealthState::Connecting, "dialing"),
    );
    let (second, _second_health) =
        HealthTestTransport::new("health-b", 2, health(TransportHealthState::Up, "ready"));
    let (mut router, _local_rx) = BACnetRouter::start(vec![
        RouterPort {
            transport: first,
            network_number: 200,
        },
        RouterPort {
            transport: second,
            network_number: 100,
        },
    ])
    .await
    .unwrap();

    let initial = router.port_health();
    assert_eq!(
        initial
            .iter()
            .map(|port| (port.config_index, port.network_number))
            .collect::<Vec<_>>(),
        vec![(0, 200), (1, 100)]
    );
    assert_eq!(initial[0].transport_kind, "health-test");
    assert_eq!(initial[0].identity, "health-a");
    assert_eq!(initial[0].health.state, TransportHealthState::Connecting);
    assert_eq!(initial[1].health.state, TransportHealthState::Up);

    first_health
        .send(health(TransportHealthState::Failed, "terminal"))
        .unwrap();
    let changed = router.port_health();
    assert_eq!(changed[0].health.state, TransportHealthState::Failed);
    assert_eq!(changed[0].health.detail.as_deref(), Some("terminal"));
    assert_eq!(initial[0].health.state, TransportHealthState::Connecting);

    router.stop().await;
}

#[tokio::test]
async fn routing_table_snapshot_contains_direct_and_learned_routes() {
    let (first, _first_health) =
        HealthTestTransport::new("routes-a", 1, health(TransportHealthState::Up, "ready"));
    let (second, _second_health) =
        HealthTestTransport::new("routes-b", 2, health(TransportHealthState::Up, "ready"));
    let (mut router, _local_rx) = BACnetRouter::start(vec![
        RouterPort {
            transport: first,
            network_number: 200,
        },
        RouterPort {
            transport: second,
            network_number: 100,
        },
    ])
    .await
    .unwrap();

    router
        .table()
        .lock()
        .await
        .add_learned(300, 1, MacAddr::from_slice(&[0xAA, 0xBB]));
    let snapshot = router.routing_table().await;
    assert_eq!(
        snapshot
            .iter()
            .map(|route| route.network_number)
            .collect::<Vec<_>>(),
        vec![100, 200, 300]
    );
    assert!(snapshot[0].directly_connected);
    assert!(snapshot[1].directly_connected);
    assert!(!snapshot[2].directly_connected);
    assert_eq!(snapshot[2].port_index, 1);
    assert_eq!(snapshot[2].next_hop_mac, vec![0xAA, 0xBB]);
    assert_eq!(snapshot[2].reachability, ReachabilityStatus::Reachable);

    router.table().lock().await.remove(300);
    assert_eq!(snapshot[2].network_number, 300);
    assert_eq!(snapshot[2].next_hop_mac, vec![0xAA, 0xBB]);

    router.stop().await;
}
