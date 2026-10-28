//! Failure and cancellation exercise the mixed session's actual native owner.
use super::*;
use bacnet_network::router::{RouterOptions, StartedRouter};
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{mpsc, oneshot};

#[derive(Default)]
struct TestPort {
    incoming: Option<mpsc::Sender<ReceivedNpdu>>,
    stopping: Option<oneshot::Sender<()>>,
    stop_gate: Option<oneshot::Receiver<()>>,
    fail_start: bool,
    fail_stop: bool,
    stops: Arc<AtomicUsize>,
    broadcasts: Option<mpsc::UnboundedSender<()>>,
}

impl TransportPort for TestPort {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        if self.fail_start {
            return Err(Error::Encoding("injected start failure".into()));
        }
        let (tx, rx) = mpsc::channel(4);
        self.incoming = Some(tx);
        Ok(rx)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = self.stopping.take() {
            let _ = entered.send(());
        }
        if let Some(gate) = self.stop_gate.take() {
            gate.await.unwrap();
        }
        self.incoming.take();
        if self.fail_stop {
            Err(Error::Encoding("injected cleanup failure".into()))
        } else {
            Ok(())
        }
    }
    async fn send_unicast(&self, _: &[u8], _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        if let Some(sent) = &self.broadcasts {
            let _ = sent.send(());
        }
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &[127, 0, 0, 1, 0, 9]
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }
}

async fn native(ports: Vec<TestPort>) -> PyResult<StartedRouter> {
    NativeRouter::start(
        ports
            .into_iter()
            .enumerate()
            .map(|(index, transport)| RouterPort {
                transport,
                network_number: index as u16 + 1,
            })
            .collect(),
        RouterOptions::new(),
    )
    .await
    .map_err(to_py_err)
}

fn session(ports: Vec<TestPort>) -> MixedSession {
    let mut session = MixedSession::new(
        Vec::new(),
        None,
        Arc::new(StdMutex::new(Vec::new())),
        Arc::new(StdMutex::new(Vec::new())),
    );
    session.test_start = Some(Box::pin(native(ports)));
    session
}

async fn start(owner: &Lifecycle<MixedSession>, ports: Vec<TestPort>) {
    owner
        .start(false, move || session(ports))
        .await
        .unwrap()
        .publish(|_| {});
}

#[tokio::test(start_paused = true)]
async fn failed_stop_is_replayed_and_blocks_restart() {
    Python::initialize();
    let owner = Lifecycle::new();
    let stops = Arc::new(AtomicUsize::new(0));
    start(
        &owner,
        vec![TestPort {
            fail_stop: true,
            stops: stops.clone(),
            ..Default::default()
        }],
    )
    .await;
    for _ in 0..2 {
        let error = close_error(owner.close().await.unwrap_err());
        Python::attach(|py| {
            assert!(error.is_instance_of::<PyRuntimeError>(py));
            assert!(error.to_string().contains("injected cleanup failure"));
        });
    }
    let restart = owner
        .start(false, || panic!("failed cleanup must fence construction"))
        .await;
    assert!(matches!(
        restart,
        Err(StartError::Cleanup(CloseError::Stop(_)))
    ));
    assert_eq!(stops.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn cancelled_start_cannot_interrupt_multiport_rollback() {
    Python::initialize();
    let owner = Lifecycle::new();
    let (entered, stopping) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    let stops = Arc::new(AtomicUsize::new(0));
    let first = TestPort {
        stopping: Some(entered),
        stop_gate: Some(gate),
        stops: stops.clone(),
        ..Default::default()
    };
    let second = TestPort {
        fail_start: true,
        ..Default::default()
    };
    let starting = tokio::spawn({
        let owner = owner.clone();
        async move {
            owner
                .start(false, move || session(vec![first, second]))
                .await
        }
    });
    stopping.await.unwrap();
    starting.abort();
    assert!(starting.await.unwrap_err().is_cancelled());
    let closing = tokio::spawn({
        let owner = owner.clone();
        async move { owner.close().await }
    });
    tokio::task::yield_now().await;
    assert!(!closing.is_finished());
    release.send(()).unwrap();
    closing.await.unwrap().unwrap();
    assert_eq!(stops.load(Ordering::SeqCst), 1);
    start(&owner, vec![TestPort::default()]).await;
    owner.close().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn cancelled_stop_keeps_cleanup_and_later_stop_is_a_barrier() {
    let owner = Lifecycle::new();
    let (entered, stopping) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    let stops = Arc::new(AtomicUsize::new(0));
    start(
        &owner,
        vec![TestPort {
            stopping: Some(entered),
            stop_gate: Some(gate),
            stops: stops.clone(),
            ..Default::default()
        }],
    )
    .await;
    let first = tokio::spawn({
        let owner = owner.clone();
        async move { owner.close().await }
    });
    stopping.await.unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let second = tokio::spawn({
        let owner = owner.clone();
        async move { owner.close().await }
    });
    tokio::task::yield_now().await;
    assert!(!second.is_finished());
    release.send(()).unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(stops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unpublished_start_result_retains_cleanup_authority() {
    let owner = Lifecycle::new();
    let publication = Publication::new(owner.clone());
    let stops = Arc::new(AtomicUsize::new(0));
    publication
        .start(session(vec![TestPort {
            stops: stops.clone(),
            ..Default::default()
        }]))
        .await
        .unwrap();
    let cleaned = owner.observe_cleanup_for_test().await;
    drop(publication);
    cleaned.await.unwrap();
    assert_eq!(stops.load(Ordering::SeqCst), 1);
    assert!(owner.inspect(|_| ()).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn sc_recovery_before_monitor_first_poll_is_announced() {
    recovery_reannouncement(false).await;
}

#[tokio::test(start_paused = true)]
async fn sc_recovery_before_monitor_spawn_is_announced() {
    recovery_reannouncement(true).await;
}

async fn recovery_reannouncement(before_spawn: bool) {
    let (sent, mut broadcasts) = mpsc::unbounded_channel();
    let started = native(vec![TestPort {
        broadcasts: Some(sent),
        ..Default::default()
    }])
    .await
    .unwrap();
    broadcasts.recv().await.unwrap(); // Startup's network number announcement.
    let router = Arc::new(Mutex::new(started.router));
    let (changes, state) = watch::channel(ScConnectionState::Connected);
    if before_spawn {
        changes.send_replace(ScConnectionState::Disconnected);
        changes.send_replace(ScConnectionState::Connected);
    }
    let tasks = spawn_background(
        &router,
        &[(
            1,
            HealthSource::Sc {
                identity: "test".into(),
                state,
            },
        )],
        None,
    );
    // No await: the monitor cannot have had its first poll yet.
    if !before_spawn {
        changes.send_replace(ScConnectionState::Disconnected);
        changes.send_replace(ScConnectionState::Connected);
    }
    tokio::time::timeout(Duration::from_secs(1), broadcasts.recv())
        .await
        .unwrap()
        .unwrap();
    for task in tasks {
        task.abort();
        let _ = task.await;
    }
    router.lock().await.stop().await.unwrap();
}
