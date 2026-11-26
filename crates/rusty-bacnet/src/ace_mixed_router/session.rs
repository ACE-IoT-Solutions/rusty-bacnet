//! Mixed transport work stays inside the shared retained lifecycle owner.
use super::*;
use crate::router::lifecycle::Session;
use bacnet_network::router::{RouterOptions, StartedRouter};

pub(super) struct MixedSession {
    configured: Vec<PyRouterPortConfig>,
    announce_interval: Option<Duration>,
    isolate_sc_ports: bool,
    running: Option<RunningRouter>,
    last_health: Arc<StdMutex<Vec<PyRouterPortHealth>>>,
    pub(super) last_counters: Arc<StdMutex<Vec<PyRouterPortCounters>>>,
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
        isolate_sc_ports: bool,
        last_health: Arc<StdMutex<Vec<PyRouterPortHealth>>>,
        last_counters: Arc<StdMutex<Vec<PyRouterPortCounters>>>,
        last_routes: Arc<StdMutex<Vec<PyRouterRouteEntry>>>,
    ) -> Self {
        Self {
            configured,
            announce_interval,
            isolate_sc_ports,
            running: None,
            last_health,
            last_counters,
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

    fn install(
        &mut self,
        started: StartedRouter,
        mut health: Vec<(u16, HealthSource)>,
        taps: Vec<FrameTap>,
    ) {
        let router = started.router;
        for (index, (_, source)) in health.iter_mut().enumerate() {
            if let HealthSource::Bip { identity } = source
                && let Some(endpoint) = bip_endpoint(&router, index)
            {
                *identity = endpoint.to_string();
            }
        }
        let frames = RouterFrames::new(&router, &self.configured, taps);
        let router = Arc::new(Mutex::new(router));
        let background = spawn_background(&router, &health, self.announce_interval);
        self.running = Some(RunningRouter {
            router,
            background,
            health,
            frames,
        });
    }
}

impl Session for MixedSession {
    type StartError = PyErr;
    type StopError = RouterStopError;

    async fn start(&mut self) -> PyResult<()> {
        #[cfg(test)]
        if let Some(start) = self.test_start.take() {
            self.install(start.await?, Vec::new(), Vec::new());
            return Ok(());
        }
        let mut built = Vec::with_capacity(self.configured.len());
        for (index, config) in self.configured.iter().cloned().enumerate() {
            built.push(build_router_port(index, config).await?);
        }
        let mut ports = Vec::with_capacity(built.len());
        let mut health = Vec::with_capacity(built.len());
        let mut taps = Vec::with_capacity(built.len());
        for (built, config) in built.into_iter().zip(&self.configured) {
            taps.push(built.tap);
            health.push((built.port.network_number, built.health));
            ports.push(
                built
                    .port
                    .isolated(self.isolate_sc_ports && matches!(config, PyRouterPortConfig::Sc(_))),
            );
        }
        let started = NativeRouter::start(ports, RouterOptions::new())
            .await
            .map_err(to_py_err)?;
        self.install(started, health, taps);
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), RouterStopError> {
        let active = self
            .running
            .as_mut()
            .expect("stop follows successful startup");
        active.frames.subscriptions.close();
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
        let counters = counter_snapshot(&router, &active.health);
        *self.last_health.locked() = health;
        *self.last_counters.locked() = counters;
        *self.last_routes.locked() = routes;
        outcome
    }
}
