//! Causal tests of the exact router owner, without Python or sockets.
#[path = "../src/router/lifecycle.rs"]
mod lifecycle;
use lifecycle::{CloseError, Lifecycle, Session, StartError};
use std::future::Future;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::oneshot;

#[derive(Default)]
struct Counts {
    live: AtomicUsize,
    stops: AtomicUsize,
}
struct Resource {
    counts: Arc<Counts>,
    starting: Option<oneshot::Sender<()>>,
    start_gate: Option<oneshot::Receiver<()>>,
    stopping: Option<oneshot::Sender<()>>,
    stop_gate: Option<oneshot::Receiver<()>>,
    start_error: bool,
    stop_error: Option<Arc<String>>,
    panic_stop: bool,
}
impl Resource {
    fn new(counts: &Arc<Counts>) -> Self {
        counts.live.fetch_add(1, Ordering::SeqCst);
        Self {
            counts: counts.clone(),
            starting: None,
            start_gate: None,
            stopping: None,
            stop_gate: None,
            start_error: false,
            stop_error: None,
            panic_stop: false,
        }
    }
}
impl Drop for Resource {
    fn drop(&mut self) {
        self.counts.live.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Session for Resource {
    type StartError = &'static str;
    type StopError = Arc<String>;
    async fn start(&mut self) -> Result<(), Self::StartError> {
        if let Some(entered) = self.starting.take() {
            entered.send(()).unwrap();
        }
        if let Some(gate) = self.start_gate.take() {
            gate.await.unwrap();
        }
        if self.start_error {
            Err("original start error")
        } else {
            Ok(())
        }
    }
    async fn stop(&mut self) -> Result<(), Self::StopError> {
        self.counts.stops.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = self.stopping.take() {
            entered.send(()).unwrap();
        }
        if let Some(gate) = self.stop_gate.take() {
            gate.await.unwrap();
        }
        assert!(!self.panic_stop, "injected stop panic");
        self.stop_error.clone().map_or(Ok(()), Err)
    }
}
type Owner = Lifecycle<Resource>;
fn starts(
    owner: &Owner,
    resource: Resource,
) -> tokio::task::JoinHandle<Result<u64, StartError<&'static str, Arc<String>>>> {
    let owner = owner.clone();
    tokio::spawn(async move { owner.start(false, move || resource).await })
}

#[tokio::test]
async fn close_barrier_and_cancelled_waiter_keep_the_admitted_owner() {
    let counts = Arc::new(Counts::default());
    let owner = Owner::new();
    let mut resource = Resource::new(&counts);
    let (entered, stopping) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    resource.stopping = Some(entered);
    resource.stop_gate = Some(gate);
    starts(&owner, resource).await.unwrap().unwrap();
    let closing = tokio::spawn({
        let owner = owner.clone();
        async move { owner.close().await }
    });
    stopping.await.unwrap();
    assert!(!closing.is_finished());
    assert_eq!(counts.live.load(Ordering::SeqCst), 1);
    closing.abort();
    assert!(closing.await.unwrap_err().is_cancelled());
    let next = tokio::spawn({
        let owner = owner.clone();
        async move { owner.close().await }
    });
    release.send(()).unwrap();
    next.await.unwrap().unwrap();
    assert_eq!(counts.live.load(Ordering::SeqCst), 0);
    assert_eq!(counts.stops.load(Ordering::SeqCst), 1);
    owner.close().await.unwrap();
    starts(&owner, Resource::new(&counts))
        .await
        .unwrap()
        .unwrap();
    owner.close().await.unwrap();
    assert_eq!(counts.stops.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cancelled_partial_start_settles_before_a_second_close_returns() {
    let counts = Arc::new(Counts::default());
    let owner = Owner::new();
    let mut resource = Resource::new(&counts);
    let (entered, starting) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    let (stop_entered, stopping) = oneshot::channel();
    let (stop_release, stop_gate) = oneshot::channel();
    resource.starting = Some(entered);
    resource.start_gate = Some(gate);
    resource.stopping = Some(stop_entered);
    resource.stop_gate = Some(stop_gate);
    let start = starts(&owner, resource);
    starting.await.unwrap();
    start.abort();
    assert!(start.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    stopping.await.unwrap();
    assert_eq!(counts.live.load(Ordering::SeqCst), 1);
    let close = tokio::spawn({
        let owner = owner.clone();
        async move { owner.close().await }
    });
    stop_release.send(()).unwrap();
    close.await.unwrap().unwrap();
    assert_eq!(counts.live.load(Ordering::SeqCst), 0);
    assert_eq!(counts.stops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn close_intent_wins_while_start_is_admitted_even_if_close_wait_is_cancelled() {
    let counts = Arc::new(Counts::default());
    let owner = Owner::new();
    let mut resource = Resource::new(&counts);
    let (entered, starting) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    resource.starting = Some(entered);
    resource.start_gate = Some(gate);
    let start = starts(&owner, resource);
    starting.await.unwrap();
    // Poll close to Pending ourselves: this proves its intent was admitted,
    // rather than assuming a spawned task ran after a scheduler yield.
    let mut close = Box::pin(owner.close());
    std::future::poll_fn(|cx| {
        assert!(close.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(close);
    release.send(()).unwrap();
    assert!(matches!(start.await.unwrap(), Err(StartError::Cancelled)));
    owner.close().await.unwrap();
    assert_eq!(counts.live.load(Ordering::SeqCst), 0);
    assert_eq!(counts.stops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_cleanup_is_replayed_without_retry_and_fences_restart() {
    let counts = Arc::new(Counts::default());
    let owner = Owner::new();
    let error = Arc::new("original non-clone cause represented by Arc".to_string());
    let mut resource = Resource::new(&counts);
    resource.stop_error = Some(error.clone());
    starts(&owner, resource).await.unwrap().unwrap();
    for _ in 0..3 {
        let Err(CloseError::Stop(actual)) = owner.close().await else {
            panic!("lost cleanup failure")
        };
        assert!(Arc::ptr_eq(&actual, &error));
    }
    assert!(matches!(
        owner
            .start(false, || panic!("must not build after failed close"))
            .await,
        Err(StartError::Cleanup(CloseError::Stop(_)))
    ));
    assert_eq!(counts.stops.load(Ordering::SeqCst), 1);
    assert_eq!(counts.live.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancelled_start_cleanup_failure_remains_observable() {
    let counts = Arc::new(Counts::default());
    let owner = Owner::new();
    let mut resource = Resource::new(&counts);
    let (entered, starting) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    resource.starting = Some(entered);
    resource.start_gate = Some(gate);
    resource.stop_error = Some(Arc::new("stop failed".into()));
    let start = starts(&owner, resource);
    starting.await.unwrap();
    start.abort();
    assert!(start.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    assert!(matches!(owner.close().await, Err(CloseError::Stop(_))));
    assert_eq!(counts.stops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn old_publication_cleanup_cannot_close_a_new_generation() {
    let counts = Arc::new(Counts::default());
    let owner = Owner::new();
    owner.close().await.unwrap();
    let first = starts(&owner, Resource::new(&counts))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        owner
            .start(true, || panic!("reentry does not rebuild"))
            .await
            .unwrap(),
        first
    );
    assert!(matches!(
        owner
            .start(false, || panic!("duplicate does not rebuild"))
            .await,
        Err(StartError::AlreadyStarted)
    ));
    owner.close().await.unwrap();
    let second = starts(&owner, Resource::new(&counts))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(first, second);
    owner.close_generation(first).await.unwrap();
    assert!(owner.inspect(|_| ()).await.is_some());
    assert_eq!(counts.stops.load(Ordering::SeqCst), 1);
    owner.close_generation(second).await.unwrap();
    assert!(owner.inspect(|_| ()).await.is_none());
    assert_eq!(counts.stops.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn start_failure_preserves_cause_and_allows_corrected_retry() {
    let counts = Arc::new(Counts::default());
    let owner = Owner::new();
    let mut resource = Resource::new(&counts);
    resource.start_error = true;
    assert!(matches!(
        starts(&owner, resource).await.unwrap(),
        Err(StartError::Operation("original start error"))
    ));
    assert_eq!(counts.live.load(Ordering::SeqCst), 0);
    starts(&owner, Resource::new(&counts))
        .await
        .unwrap()
        .unwrap();
    owner.close().await.unwrap();
}

#[tokio::test]
async fn disposal_fences_admitted_publication_and_future_starts() {
    let counts = Arc::new(Counts::default());
    let owner = Owner::new();
    let mut resource = Resource::new(&counts);
    let (entered, starting) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    resource.starting = Some(entered);
    resource.start_gate = Some(gate);
    let start = starts(&owner, resource);
    starting.await.unwrap();
    owner.dispose();
    release.send(()).unwrap();
    assert!(matches!(start.await.unwrap(), Err(StartError::Cancelled)));
    owner.close().await.unwrap();
    assert!(matches!(
        owner.start(false, || panic!("disposed")).await,
        Err(StartError::Cancelled)
    ));
    assert_eq!(counts.live.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn worker_panic_never_becomes_success_or_admits_a_restart() {
    let counts = Arc::new(Counts::default());
    let owner = Owner::new();
    let mut resource = Resource::new(&counts);
    resource.panic_stop = true;
    starts(&owner, resource).await.unwrap().unwrap();
    assert!(matches!(owner.close().await, Err(CloseError::WorkerEnded)));
    assert!(matches!(owner.close().await, Err(CloseError::WorkerEnded)));
    assert!(matches!(
        owner.start(false, || panic!("must not restart")).await,
        Err(StartError::Cleanup(CloseError::WorkerEnded))
    ));
    assert_eq!(counts.stops.load(Ordering::SeqCst), 1);
}
