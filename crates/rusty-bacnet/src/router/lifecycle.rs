//! Router-only admission and retained startup/cleanup, independent of Python.
//!
//! The admitted worker owns the state lock through publication or cleanup.
//! Failed cleanup is terminal for this owner; no later start can discard it.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{Mutex, oneshot};

pub(crate) trait Session: Send + 'static {
    type StartError: Send + 'static;
    type StopError: Clone + Send + Sync + 'static;
    fn start(&mut self) -> impl Future<Output = Result<(), Self::StartError>> + Send;
    fn stop(&mut self) -> impl Future<Output = Result<(), Self::StopError>> + Send;
}

#[derive(Clone, Debug)]
pub(crate) enum CloseError<E> {
    Stop(E),
    WorkerEnded,
}

#[derive(Debug)]
pub(crate) enum StartError<E, C> {
    AlreadyStarted,
    Cancelled,
    Operation(E),
    Cleanup(CloseError<C>),
}

enum Phase<S: Session> {
    Idle,
    Starting,
    Running(S),
    Closing,
    Failed(CloseError<S::StopError>),
}

struct State<S: Session> {
    generation: u64,
    phase: Phase<S>,
}

pub(crate) struct Lifecycle<S: Session> {
    state: Arc<Mutex<State<S>>>,
    closing: Arc<AtomicUsize>,
    disposed: Arc<AtomicBool>,
}

impl<S: Session> Clone for Lifecycle<S> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            closing: self.closing.clone(),
            disposed: self.disposed.clone(),
        }
    }
}

struct CloseIntent(Arc<AtomicUsize>);
impl Drop for CloseIntent {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl<S: Session> Lifecycle<S> {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                generation: 0,
                phase: Phase::Idle,
            })),
            closing: Arc::new(AtomicUsize::new(0)),
            disposed: Arc::new(AtomicBool::new(false)),
        }
    }

    // Private fixture for exercising the real Python context methods against a
    // native transport cleanup failure. No injection surface exists in Python.
    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "the standalone owner harness has no Python fixture"
    )]
    pub(crate) fn with_running_for_test(running: S) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                generation: 1,
                phase: Phase::Running(running),
            })),
            closing: Arc::new(AtomicUsize::new(0)),
            disposed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Prevent queued or future starts before a destructor schedules cleanup.
    pub(crate) fn dispose(&self) {
        self.disposed.store(true, Ordering::SeqCst);
    }

    pub(crate) async fn start<F>(
        &self,
        reenter: bool,
        build: F,
    ) -> Result<u64, StartError<S::StartError, S::StopError>>
    where
        F: FnOnce() -> S + Send + 'static,
    {
        let mut state = self.state.clone().lock_owned().await;
        if self.disposed.load(Ordering::SeqCst) || self.closing.load(Ordering::SeqCst) > 0 {
            return Err(StartError::Cancelled);
        }
        match &state.phase {
            Phase::Running(_) => {
                return if reenter {
                    Ok(state.generation)
                } else {
                    Err(StartError::AlreadyStarted)
                };
            }
            Phase::Failed(error) => return Err(StartError::Cleanup(error.clone())),
            Phase::Starting | Phase::Closing => {
                return Err(StartError::Cleanup(CloseError::WorkerEnded));
            }
            Phase::Idle => {}
        }
        state.generation += 1;
        let generation = state.generation;
        state.phase = Phase::Starting;
        let closing = self.closing.clone();
        let disposed = self.disposed.clone();
        let (result, waiter) = oneshot::channel();
        // Actual multiport startup lives inside this retained task. There is
        // no cancellable preparation phase containing transport activity.
        tokio::spawn(async move {
            let mut running = build();
            if let Err(error) = running.start().await {
                state.phase = Phase::Idle;
                let _ = result.send(Err(StartError::Operation(error)));
                return;
            }
            if result.is_closed()
                || closing.load(Ordering::SeqCst) > 0
                || disposed.load(Ordering::SeqCst)
            {
                let outcome = settle(&mut state, running).await;
                let _ = result.send(match outcome {
                    Ok(()) => Err(StartError::Cancelled),
                    Err(error) => Err(StartError::Cleanup(error)),
                });
                return;
            }
            state.phase = Phase::Running(running);
            if result.send(Ok(generation)).is_err() {
                let Phase::Running(running) = std::mem::replace(&mut state.phase, Phase::Closing)
                else {
                    unreachable!()
                };
                let _ = settle(&mut state, running).await;
            }
        });
        waiter
            .await
            .unwrap_or(Err(StartError::Cleanup(CloseError::WorkerEnded)))
    }

    pub(crate) async fn inspect<R>(&self, read: impl FnOnce(&S) -> R) -> Option<R> {
        let state = self.state.lock().await;
        if self.closing.load(Ordering::SeqCst) > 0 || self.disposed.load(Ordering::SeqCst) {
            return None;
        }
        match &state.phase {
            Phase::Running(running) => Some(read(running)),
            _ => None,
        }
    }

    pub(crate) async fn close(&self) -> Result<(), CloseError<S::StopError>> {
        self.close_inner(None).await
    }

    /// A result lost by asyncio only cleans its own generation, never a later restart.
    pub(crate) async fn close_generation(
        &self,
        generation: u64,
    ) -> Result<(), CloseError<S::StopError>> {
        self.close_inner(Some(generation)).await
    }

    async fn close_inner(&self, generation: Option<u64>) -> Result<(), CloseError<S::StopError>> {
        let intent = if generation.is_none() {
            self.closing.fetch_add(1, Ordering::SeqCst);
            Some(CloseIntent(self.closing.clone()))
        } else {
            None
        };
        let state = self.state.clone();
        let (result, waiter) = oneshot::channel();
        // Intent and the admission wait belong to the worker too. Cancelling
        // the caller cannot withdraw an already requested close.
        tokio::spawn(async move {
            let intent = intent;
            let mut state = state.lock_owned().await;
            if generation.is_some_and(|generation| generation != state.generation) {
                drop(state);
                drop(intent);
                let _ = result.send(Ok(()));
                return;
            }
            let outcome = match std::mem::replace(&mut state.phase, Phase::Closing) {
                Phase::Running(running) => settle(&mut state, running).await,
                Phase::Idle => {
                    state.phase = Phase::Idle;
                    Ok(())
                }
                Phase::Failed(error) => {
                    state.phase = Phase::Failed(error.clone());
                    Err(error)
                }
                Phase::Starting | Phase::Closing => {
                    state.phase = Phase::Failed(CloseError::WorkerEnded);
                    Err(CloseError::WorkerEnded)
                }
            };
            drop(state);
            drop(intent);
            let _ = result.send(outcome);
        });
        waiter.await.unwrap_or(Err(CloseError::WorkerEnded))
    }
}

async fn settle<S: Session>(
    state: &mut State<S>,
    mut running: S,
) -> Result<(), CloseError<S::StopError>> {
    state.phase = Phase::Closing;
    match running.stop().await {
        Ok(()) => {
            state.phase = Phase::Idle;
            Ok(())
        }
        Err(error) => {
            let error = CloseError::Stop(error);
            state.phase = Phase::Failed(error.clone());
            Err(error)
        }
    }
}
