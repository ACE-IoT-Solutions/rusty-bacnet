use super::*;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket as Socket};
use std::pin::Pin;
use std::task::{Context, Waker};
use tokio::sync::oneshot;

fn socket(port: u16) -> std::io::Result<Socket> {
    Socket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
}

fn available() -> u16 {
    socket(0).unwrap().local_addr().unwrap().port()
}

fn bound(port: u16) {
    assert_eq!(
        socket(port).unwrap_err().kind(),
        std::io::ErrorKind::AddrInUse
    );
}

fn after_start(
    runtime: &BacnetRuntime,
    id: u128,
) -> (
    oneshot::Receiver<Arc<ManagedAttachment>>,
    oneshot::Sender<()>,
) {
    let (entered, seen) = oneshot::channel();
    let (resume, gate) = oneshot::channel();
    runtime
        .inner
        .started
        .lock()
        .unwrap()
        .insert(id.into(), (entered, gate));
    (seen, resume)
}

fn during_stop(
    owner: &ManagedAttachment,
) -> (
    oneshot::Receiver<()>,
    oneshot::Sender<Result<(), bacnet_types::error::Error>>,
) {
    let (entered, seen) = oneshot::channel();
    let (resume, gate) = oneshot::channel();
    *owner.stop_barrier.lock().unwrap() = Some((entered, gate));
    (seen, resume)
}

fn pending<F: Future>(future: Pin<&mut F>) {
    assert!(
        future
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
}

async fn assert_fenced(runtime: &BacnetRuntime, retained: u128) {
    let health = runtime.health().await;
    assert!(!health.accepting_commands);
    assert_eq!(health.generation, 1);
    assert_eq!(runtime.inner.revision.load(Ordering::Acquire), 1);
    assert_eq!(health.attachment_task_count, 1);
    assert_eq!(health.attachments[0].id, retained.into());
    assert_eq!(health.attachments[0].state, AttachmentState::Failed);
    let error = runtime
        .read_batch(ReadBatch {
            items: vec![],
            timeout: Duration::from_secs(1),
        })
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, crate::ErrorCode::Cancelled);
    assert_eq!(
        runtime.reconcile(1, vec![]).await.unwrap_err().code,
        crate::ErrorCode::Cancelled
    );
}

#[tokio::test(start_paused = true)]
async fn cancelled_stop_then_deadline_resumes_the_original_shutdown() {
    let port = available();
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, port)],
        ..Default::default()
    })
    .await
    .unwrap();
    let owner = runtime.inner.attachment(1.into()).unwrap();
    let (entered, resume) = during_stop(&owner);
    let mut stop = Box::pin(runtime.stop());
    pending(stop.as_mut());
    entered.await.unwrap();
    drop(stop);
    assert_eq!(
        runtime.health().await.attachments[0].state,
        AttachmentState::Stopping
    );
    bound(port);
    let error = runtime.stop().await.unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::DeadlineExceeded);
    assert_fenced(&runtime, 1).await;
    bound(port);
    resume.send(Ok(())).unwrap();
    let report = runtime.stop().await.unwrap();
    assert_eq!(report.tasks_joined, 1);
    assert_eq!(report.remaining_attachments, 0);
    drop(socket(port).unwrap());
}

#[tokio::test(start_paused = true)]
async fn old_stop_failure_releases_confirmed_owners_and_never_rebinds_them() {
    let port = available();
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, port), attachment(2, 0)],
        ..Default::default()
    })
    .await
    .unwrap();
    let owner = runtime.inner.attachment(2.into()).unwrap();
    let (entered, resume) = during_stop(&owner);
    let task = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.reconcile(2, vec![attachment(3, port)]).await })
    };
    entered.await.unwrap();
    resume
        .send(Err(bacnet_types::error::Error::Transport(
            std::io::Error::other("injected stop failure"),
        )))
        .unwrap();
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .message
            .contains("injected stop failure")
    );
    assert_fenced(&runtime, 2).await;
    drop(socket(port).unwrap());
    assert_eq!(runtime.stop().await.unwrap().tasks_joined, 1);
}

#[tokio::test(start_paused = true)]
async fn failed_desired_cleanup_retains_partial_owner_and_primary_error() {
    let old = available();
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, old)],
        ..Default::default()
    })
    .await
    .unwrap();
    let occupied = socket(0).unwrap();
    let busy = occupied.local_addr().unwrap().port();
    let partial = available();
    let (started, release) = after_start(&runtime, 2);
    let task = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .reconcile(2, vec![attachment(2, partial), attachment(3, busy)])
                .await
        })
    };
    let owner = started.await.unwrap();
    let (stopping, finish) = during_stop(&owner);
    release.send(()).unwrap();
    stopping.await.unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::RollbackFailed);
    assert!(
        error.message.contains("BindConflict")
            && error.message.contains("replacement cleanup")
            && error.message.contains("DeadlineExceeded")
    );
    assert_fenced(&runtime, 2).await;
    bound(partial);
    drop(socket(old).unwrap());
    finish.send(Ok(())).unwrap();
    runtime.stop().await.unwrap();
    drop(socket(partial).unwrap());
}

#[tokio::test(start_paused = true)]
async fn failed_rollback_cleanup_retains_owner_and_all_three_failures() {
    let original = available();
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, 0), attachment(2, original)],
        ..Default::default()
    })
    .await
    .unwrap();
    let occupied = socket(0).unwrap();
    let busy = occupied.local_addr().unwrap().port();
    let (replacement, release) = after_start(&runtime, 3);
    let (restored, resume_restore) = after_start(&runtime, 1);
    let task = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .reconcile(2, vec![attachment(3, 0), attachment(4, busy)])
                .await
        })
    };
    replacement.await.unwrap();
    let collision = socket(original).unwrap();
    release.send(()).unwrap();
    let owner = restored.await.unwrap();
    let (stopping, finish) = during_stop(&owner);
    resume_restore.send(()).unwrap();
    stopping.await.unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::RollbackFailed);
    assert_eq!(error.message.matches("BindConflict").count(), 2);
    assert!(
        error.message.contains("rollback cleanup") && error.message.contains("DeadlineExceeded")
    );
    assert_fenced(&runtime, 1).await;
    finish.send(Ok(())).unwrap();
    runtime.stop().await.unwrap();
    drop(collision);
}

#[tokio::test]
async fn cancelling_reconcile_after_old_stop_finishes_rollback_and_serves_a_read() {
    let old = available();
    let previous = vec![attachment(1, old)];
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: previous.clone(),
        ..Default::default()
    })
    .await
    .unwrap();
    let occupied = socket(0).unwrap();
    let busy = occupied.local_addr().unwrap().port();
    let (started, release) = after_start(&runtime, 2);
    let task = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .reconcile(2, vec![attachment(2, 0), attachment(3, busy)])
                .await
        })
    };
    started.await.unwrap();
    drop(socket(old).unwrap());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    assert!(runtime.reconcile(1, previous).await.unwrap().idempotent);
    bound(old);
    assert_read(&runtime).await;
    runtime.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn lost_initial_start_cleans_partial_and_offered_unclaimed_owners() {
    for after_offer in [false, true] {
        let port = available();
        let runtime = BacnetRuntime::unstarted(RuntimeConfig {
            attachments: vec![attachment(1, port), attachment(2, 0)],
            ..Default::default()
        });
        let (started, release) = after_start(&runtime, 1);
        let (offered, offer_seen) = oneshot::channel();
        *runtime.inner.offered.lock().unwrap() = Some(offered);
        let (done, cleaned) = oneshot::channel();
        *runtime.inner.cleaned.lock().unwrap() = Some(done);
        let mut start = Box::pin(runtime.initialize());
        pending(start.as_mut());
        started.await.unwrap();
        bound(port);
        if after_offer {
            release.send(()).unwrap();
            offer_seen.await.unwrap();
            drop(start);
        } else {
            drop(start);
            release.send(()).unwrap();
        }
        cleaned.await.unwrap();
        drop(socket(port).unwrap());
    }
}

#[tokio::test]
async fn concurrent_guards_publish_the_authoritative_empty_operation_map() {
    let runtime = BacnetRuntime::start(RuntimeConfig::default())
        .await
        .unwrap();
    let count = 8;
    let barrier = std::sync::Barrier::new(count);
    std::thread::scope(|scope| {
        for id in 0..count {
            let guard = runtime
                .reserve_operation(OperationId(id as u64), runtime.inner.clone())
                .unwrap();
            let ready = &barrier;
            scope.spawn(move || {
                ready.wait();
                drop(guard);
            });
        }
    });
    assert!(runtime.inner.operations.lock().unwrap().is_empty());
    assert_eq!(*runtime.inner.operation_count.borrow(), 0);
    runtime.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn operation_drain_timeout_fences_work_and_retry_joins_parked_shutdown() {
    let port = available();
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, port)],
        ..Default::default()
    })
    .await
    .unwrap();
    let guard = runtime
        .reserve_operation(OperationId(98), runtime.inner.clone())
        .unwrap();
    let owner = runtime.inner.attachment(1.into()).unwrap();
    let (_entered, finish) = during_stop(&owner);
    let error = runtime.stop().await.unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::DeadlineExceeded);
    assert!(
        error.message.contains("operation shutdown timed out")
            && error.message.contains("attachment shutdown timed out")
    );
    assert_eq!(runtime.health().await.operation_count, 1);
    assert_fenced(&runtime, 1).await;
    bound(port);
    drop(guard);
    finish.send(Ok(())).unwrap();
    assert_eq!(runtime.stop().await.unwrap().tasks_joined, 1);
    assert_eq!(runtime.health().await.operation_count, 0);
    drop(socket(port).unwrap());
}

#[tokio::test(start_paused = true)]
async fn cancelled_waiter_can_observe_its_committed_revision() {
    let runtime = BacnetRuntime::start(RuntimeConfig::default())
        .await
        .unwrap();
    let configured = attachment(1, available());
    let desired = vec![configured];
    let (started, resume) = after_start(&runtime, 1);
    let mut transition = Box::pin(runtime.reconcile(2, desired.clone()));
    pending(transition.as_mut());
    started.await.unwrap();
    drop(transition);
    resume.send(()).unwrap();
    let committed = runtime.reconcile(2, desired).await.unwrap();
    assert!(committed.idempotent);
    assert_eq!((committed.revision, committed.generation), (2, 2));
    assert_eq!(
        runtime.reconcile(1, vec![]).await.unwrap_err().code,
        crate::ErrorCode::StaleRevision
    );
    runtime.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn failed_initial_cleanup_reports_both_causes_and_finishes_without_a_runtime_handle() {
    let port = available();
    let busy = socket(0).unwrap();
    let runtime = BacnetRuntime::unstarted(RuntimeConfig {
        attachments: vec![
            attachment(1, port),
            attachment(2, busy.local_addr().unwrap().port()),
        ],
        ..Default::default()
    });
    let (started, resume) = after_start(&runtime, 1);
    let (done, cleaned) = oneshot::channel();
    *runtime.inner.cleaned.lock().unwrap() = Some(done);
    let task = tokio::spawn(runtime.initialize());
    let owner = started.await.unwrap();
    let (stopping, finish) = during_stop(&owner);
    resume.send(()).unwrap();
    stopping.await.unwrap();
    let error = task.await.unwrap().err().unwrap();
    assert_eq!(error.code, crate::ErrorCode::RollbackFailed);
    assert!(
        error.message.contains("BindConflict")
            && error.message.contains("startup cleanup")
            && error.message.contains("DeadlineExceeded")
    );
    bound(port);
    finish.send(Ok(())).unwrap();
    cleaned.await.unwrap();
    drop(socket(port).unwrap());
}

#[tokio::test(start_paused = true)]
async fn startup_worker_loss_joins_its_partial_owner() {
    let port = available();
    let runtime = BacnetRuntime::unstarted(RuntimeConfig {
        attachments: vec![attachment(1, port)],
        ..Default::default()
    });
    let (started, resume) = after_start(&runtime, 1);
    let (done, cleaned) = oneshot::channel();
    *runtime.inner.cleaned.lock().unwrap() = Some(done);
    let task = tokio::spawn(runtime.initialize());
    let owner = started.await.unwrap();
    let (entered, finish) = during_stop(&owner);
    drop(resume);
    let error = task.await.unwrap().err().unwrap();
    assert_eq!(error.code, crate::ErrorCode::Internal);
    entered.await.unwrap();
    bound(port);
    finish.send(Ok(())).unwrap();
    cleaned.await.unwrap();
    drop(socket(port).unwrap());
}
