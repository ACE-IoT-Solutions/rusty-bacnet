//! Mixed transport work stays inside the shared retained lifecycle owner.
use super::*;
use crate::router::lifecycle::Session;
use bacnet_network::router::{RouterOptions, StartedRouter};

pub(super) struct MixedSession {
    configured: Vec<PyRouterPortConfig>,
    announce_interval: Option<Duration>,
    running: Option<RunningRouter>,
    last_health: Arc<StdMutex<Vec<PyRouterPortHealth>>>,
    last_routes: Arc<StdMutex<Vec<PyRouterRouteEntry>>>,
    #[cfg(test)]
    pub(super) test_start: Option<
        std::pin::Pin<Box<dyn std::future::Future<Output = PyResult<StartedRouter>> + Send>>,
    >,
}

impl MixedSession {
    pub(super) fn new(
        configured: Vec<PyRouterPortConfig>,
        announce_interval: Option<Duration>,
        last_health: Arc<StdMutex<Vec<PyRouterPortHealth>>>,
        last_routes: Arc<StdMutex<Vec<PyRouterRouteEntry>>>,
    ) -> Self {
        Self {
            configured,
            announce_interval,
            running: None,
            last_health,
            last_routes,
            #[cfg(test)]
            test_start: None,
        }
    }

    pub(super) fn active(&self) -> &RunningRouter {
        self.running
            .as_ref()
            .expect("running session was published")
    }

    fn install(&mut self, started: StartedRouter, mut health: Vec<(u16, HealthSource)>) {
        let router = started.router;
        for (index, (_, source)) in health.iter_mut().enumerate() {
            if let HealthSource::Bip { identity } = source
                && let Some(endpoint) = bip_endpoint(&router, index)
            {
                *identity = endpoint.to_string();
            }
        }
        let router = Arc::new(Mutex::new(router));
        let background = spawn_background(&router, &health, self.announce_interval);
        self.running = Some(RunningRouter {
            router,
            background,
            health,
        });
    }
}

impl Session for MixedSession {
    type StartError = PyErr;
    type StopError = RouterStopError;

    async fn start(&mut self) -> PyResult<()> {
        #[cfg(test)]
        if let Some(start) = self.test_start.take() {
            self.install(start.await?, Vec::new());
            return Ok(());
        }
        let mut built = Vec::with_capacity(self.configured.len());
        for (index, config) in self.configured.iter().cloned().enumerate() {
            built.push(build_router_port(index, config).await?);
        }
        let mut ports = Vec::with_capacity(built.len());
        let mut health = Vec::with_capacity(built.len());
        for built in built {
            health.push((built.port.network_number, built.health));
            ports.push(built.port);
        }
        let started = NativeRouter::start(ports, RouterOptions::new())
            .await
            .map_err(to_py_err)?;
        self.install(started, health);
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), RouterStopError> {
        let active = self
            .running
            .as_mut()
            .expect("stop follows successful startup");
        for task in &active.background {
            task.abort();
        }
        for task in active.background.drain(..) {
            let _ = task.await;
        }
        let health = health_snapshot(&active.health);
        let mut router = active.router.lock().await;
        let routes = route_snapshot(&router);
        let outcome = router.stop().await;
        *self.last_health.lock().unwrap_or_else(|e| e.into_inner()) = health;
        *self.last_routes.lock().unwrap_or_else(|e| e.into_inner()) = routes;
        outcome
    }
}
