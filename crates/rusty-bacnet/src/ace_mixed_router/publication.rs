//! Keep cleanup authority until asyncio accepts this generation's startup.
use super::*;
use crate::py_async::{PutBack, Via};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub(super) struct Publication {
    owner: Lifecycle<MixedSession>,
    generation: AtomicU64,
    settled: AtomicBool,
    cleaning: AtomicBool,
}

impl Publication {
    pub(super) fn new(owner: Lifecycle<MixedSession>) -> Arc<Self> {
        Arc::new(Self {
            owner,
            generation: AtomicU64::new(0),
            settled: AtomicBool::new(false),
            cleaning: AtomicBool::new(false),
        })
    }

    pub(super) async fn start(
        &self,
        session: MixedSession,
    ) -> Result<(), StartError<PyErr, RouterStopError>> {
        self.owner
            .start(false, move || session)
            .await?
            .publish(|generation| {
                self.generation.store(generation, Ordering::SeqCst);
            });
        Ok(())
    }

    fn cleanup(&self) {
        let generation = self.generation.load(Ordering::SeqCst);
        if generation != 0 && !self.cleaning.swap(true, Ordering::SeqCst) {
            let owner = self.owner.clone();
            crate::py_async::spawn_cleanup(async move {
                let _ = owner.close_generation(generation).await;
            });
        }
    }
}

impl PutBack for Publication {
    fn put_back(&self, _: Python<'_>, _: Py<PyAny>, _: Via) {
        self.cleanup();
    }

    fn settled(&self) {
        self.settled.store(true, Ordering::SeqCst);
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        if !self.settled.load(Ordering::SeqCst) {
            self.cleanup();
        }
    }
}
