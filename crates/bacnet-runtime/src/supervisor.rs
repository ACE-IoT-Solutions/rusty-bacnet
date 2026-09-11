use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{watch, Mutex};
use tokio::task::JoinSet;

struct LiveTaskGuard(Arc<AtomicUsize>);

impl Drop for LiveTaskGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug)]
pub(crate) struct Supervisor {
    accepting: AtomicBool,
    live_tasks: Arc<AtomicUsize>,
    stop_tx: watch::Sender<bool>,
    tasks: Mutex<JoinSet<()>>,
}

impl Supervisor {
    pub(crate) fn new() -> Self {
        let (stop_tx, _) = watch::channel(false);
        Self {
            accepting: AtomicBool::new(true),
            live_tasks: Arc::new(AtomicUsize::new(0)),
            stop_tx,
            tasks: Mutex::new(JoinSet::new()),
        }
    }

    pub(crate) async fn spawn<F, Fut>(&self, task: F) -> bool
    where
        F: FnOnce(watch::Receiver<bool>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let mut tasks = self.tasks.lock().await;
        while tasks.try_join_next().is_some() {}
        if !self.accepting.load(Ordering::Acquire) {
            return false;
        }
        let stop_rx = self.stop_tx.subscribe();
        self.live_tasks.fetch_add(1, Ordering::AcqRel);
        let live_tasks = Arc::clone(&self.live_tasks);
        tasks.spawn(async move {
            let _guard = LiveTaskGuard(live_tasks);
            task(stop_rx).await;
        });
        true
    }

    pub(crate) fn accepting(&self) -> bool {
        self.accepting.load(Ordering::Acquire)
    }

    pub(crate) fn task_count(&self) -> usize {
        self.live_tasks.load(Ordering::Acquire)
    }

    pub(crate) async fn stop(&self, timeout: Duration) -> (usize, usize) {
        let mut tasks = self.tasks.lock().await;
        self.accepting.store(false, Ordering::Release);
        let _ = self.stop_tx.send(true);
        let deadline = tokio::time::Instant::now() + timeout;
        let mut joined = 0;
        while !tasks.is_empty() {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, tasks.join_next()).await {
                Ok(Some(_)) => {
                    joined += 1;
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        let aborted = tasks.len();
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        (joined, aborted)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::Supervisor;

    #[tokio::test]
    async fn concurrent_spawn_and_stop_cannot_leave_a_task_behind() {
        for _ in 0..100 {
            let supervisor = Arc::new(Supervisor::new());
            let spawn_supervisor = Arc::clone(&supervisor);
            let spawn = tokio::spawn(async move {
                spawn_supervisor
                    .spawn(|mut stop| async move {
                        let _ = stop.changed().await;
                    })
                    .await
            });
            let stop_supervisor = Arc::clone(&supervisor);
            let stop =
                tokio::spawn(async move { stop_supervisor.stop(Duration::from_secs(1)).await });

            let _accepted = spawn.await.unwrap();
            let _report = stop.await.unwrap();
            assert!(!supervisor.accepting());
            assert_eq!(supervisor.task_count(), 0);
            assert!(!supervisor.spawn(|_| async {}).await);
        }
    }

    #[tokio::test]
    async fn completed_short_lived_tasks_are_reaped_before_the_next_spawn() {
        let supervisor = Supervisor::new();
        for _ in 0..10_000 {
            assert!(supervisor.spawn(|_| async {}).await);
            tokio::task::yield_now().await;
        }

        while supervisor.task_count() != 0 {
            tokio::task::yield_now().await;
        }
        assert!(supervisor.tasks.lock().await.len() <= 1);

        let (joined, aborted) = supervisor.stop(Duration::from_secs(1)).await;
        assert!(joined <= 1);
        assert_eq!(aborted, 0);
    }
}
