//! Reconnect policy and bounded ownership of retired node sockets.

use super::connector::{dial_failover_ws_result, dial_reconnect_ws};
use super::*;
use crate::port::{TransportHealth, TransportHealthState};

pub(super) async fn retire<W: WebSocketPort>(
    current_ws: &Arc<W>,
    primary_ws: &mut Option<Arc<W>>,
    conn: &Arc<Mutex<ScConnection>>,
    state_tx: &watch::Sender<ScConnectionState>,
    restore_disconnect_task: &Arc<StdMutex<Option<JoinHandle<()>>>>,
) {
    // Seal new public send/stop admission before any recovery await. The NAK
    // future has already been dropped. Previously admitted application sends
    // may finish; neither their writes nor buffered NAK bytes are rolled back.
    {
        let mut c = conn.lock().await;
        c.state = ScConnectionState::Disconnected;
        state_tx.send_replace(c.state);
    }
    if primary_ws
        .as_ref()
        .is_some_and(|ws| Arc::ptr_eq(ws, current_ws))
    {
        *primary_ws = None;
    }
    // A restore task targets the previous failover, not the current socket
    // (connector freshness is a caller contract). Terminate any outstanding
    // task as well, so it cannot initiate deferred cleanup after retirement.
    let task = restore_disconnect_task
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(task) = task {
        task.abort();
        let _ = task.await;
    }
}

pub(super) struct Recovery<'a, W: WebSocketPort> {
    pub config: &'a ScReconnectConfig,
    pub primary_connector: &'a Option<WebSocketConnector<W>>,
    pub failover_connector: &'a Option<WebSocketConnector<W>>,
    pub failover_ws: &'a mut Option<Arc<W>>,
    pub conn: &'a Arc<Mutex<ScConnection>>,
    pub active_ws: &'a Arc<Mutex<Arc<W>>>,
    pub state_tx: &'a watch::Sender<ScConnectionState>,
    pub health_tx: &'a watch::Sender<TransportHealth>,
    pub connect_timeout_ms: u64,
    pub effective_max_apdu_length: &'a AtomicU16,
}

impl<W: WebSocketPort> Recovery<'_, W> {
    pub(super) async fn reconnect(
        &mut self,
        current_ws: &Arc<W>,
        active_hub: ActiveHub,
        current_reusable: bool,
    ) -> Option<(Arc<W>, ActiveHub)> {
        warn!("SC transport disconnected, attempting reconnection");
        let mut backoff = Duration::from_millis(self.config.initial_delay_ms);
        let max_backoff = Duration::from_millis(self.config.max_delay_ms);
        let mut attempt = 0u64;
        let alternate_available = match active_hub {
            ActiveHub::Primary => self.failover_connector.is_some() || self.failover_ws.is_some(),
            ActiveHub::Failover => self.primary_connector.is_some(),
        };
        loop {
            if !self.config.retry_forever && attempt >= u64::from(self.config.max_retries) {
                break;
            }
            attempt += 1;
            let target_hub = if self.config.retry_forever && alternate_available && attempt % 2 == 0
            {
                match active_hub {
                    ActiveHub::Primary => ActiveHub::Failover,
                    ActiveHub::Failover => ActiveHub::Primary,
                }
            } else {
                active_hub
            };
            let last_error = self.health_tx.borrow().last_error.clone();
            self.health_tx.send_replace(TransportHealth {
                state: TransportHealthState::Reconnecting,
                detail: Some("reconnecting".into()),
                active_hub: Some(target_hub.label().into()),
                last_error,
                attempt,
                since: None,
            });
            tokio::time::sleep(backoff).await;
            if !self.conn.lock().await.connect_retry_allowed {
                warn!(attempt, "SC reconnection skipped without retry eligibility");
                self.health_tx.send_modify(|health| {
                    health.last_error = Some("hub response forbids reconnect".into());
                });
                break;
            }
            {
                let mut c = self.conn.lock().await;
                c.reset_for_connect_retry();
                self.state_tx.send_replace(c.state);
            }
            let dialed = if target_hub == ActiveHub::Failover && target_hub != active_hub {
                dial_failover_ws_result(
                    self.failover_connector,
                    self.failover_ws,
                    self.connect_timeout_ms,
                )
                .await
            } else {
                dial_reconnect_ws(
                    target_hub,
                    self.primary_connector,
                    self.failover_connector,
                    self.connect_timeout_ms,
                )
                .await
            };
            let reconnect_ws = match dialed {
                Ok(Some(ws)) => ws,
                Ok(None) if target_hub == active_hub && current_reusable => current_ws.clone(),
                Ok(None) => {
                    let message = format!("SC {} hub has no fresh connector", target_hub.label());
                    warn!("{message}");
                    self.health_tx
                        .send_modify(|health| health.last_error = Some(message));
                    if self.config.retry_forever {
                        backoff = (backoff * 2).min(max_backoff);
                        continue;
                    }
                    break;
                }
                Err(e) => {
                    warn!(%e, attempt, "SC reconnection redial failed");
                    self.health_tx
                        .send_modify(|health| health.last_error = Some(e.to_string()));
                    backoff = (backoff * 2).min(max_backoff);
                    continue;
                }
            };
            let probe_conn = connect_probe_from(self.conn).await;
            match perform_handshake(&*reconnect_ws, &probe_conn, None, self.connect_timeout_ms)
                .await
            {
                Ok(()) => {
                    self.publish(&reconnect_ws, &probe_conn).await;
                    self.health_tx.send_replace(TransportHealth {
                        state: TransportHealthState::Up,
                        detail: Some("connected".into()),
                        active_hub: Some(target_hub.label().into()),
                        last_error: None,
                        attempt,
                        since: Some(Instant::now()),
                    });
                    info!(attempt, "SC reconnected after backoff");
                    return Some((reconnect_ws, target_hub));
                }
                Err(e) => {
                    absorb_failed_connect_probe(self.conn, &probe_conn).await;
                    self.health_tx
                        .send_modify(|health| health.last_error = Some(e.to_string()));
                    if !self.conn.lock().await.connect_retry_allowed {
                        warn!(%e, attempt, "SC reconnection failed without retry eligibility");
                        break;
                    }
                    warn!(%e, attempt, "SC reconnection failed, retrying in {:?}", backoff);
                    backoff = (backoff * 2).min(max_backoff);
                }
            }
        }
        // Preserve the existing order: only primary exhaustion tries failover.
        // An untouched preconfigured failover is consumed once, never poisoned
        // merely because a different (primary) socket was retired.
        if !self.config.retry_forever
            && active_hub == ActiveHub::Primary
            && self.conn.lock().await.connect_retry_allowed
        {
            match dial_failover_ws_result(
                self.failover_connector,
                self.failover_ws,
                self.connect_timeout_ms,
            )
            .await
            {
                Ok(Some(failover)) => {
                    warn!("SC primary reconnection exhausted, attempting failover hub");
                    {
                        let mut c = self.conn.lock().await;
                        c.reset_for_connect_retry();
                        self.state_tx.send_replace(c.state);
                    }
                    let probe_conn = connect_probe_from(self.conn).await;
                    match perform_handshake(&*failover, &probe_conn, None, self.connect_timeout_ms)
                        .await
                    {
                        Ok(()) => {
                            self.publish(&failover, &probe_conn).await;
                            self.health_tx.send_replace(TransportHealth {
                                state: TransportHealthState::Up,
                                detail: Some("connected".into()),
                                active_hub: Some(ActiveHub::Failover.label().into()),
                                last_error: None,
                                attempt: attempt.saturating_add(1),
                                since: Some(Instant::now()),
                            });
                            info!(
                                "SC connected to failover hub after primary reconnect exhaustion"
                            );
                            return Some((failover, ActiveHub::Failover));
                        }
                        Err(e) => {
                            absorb_failed_connect_probe(self.conn, &probe_conn).await;
                            self.health_tx
                                .send_modify(|health| health.last_error = Some(e.to_string()));
                            warn!(%e, "SC failover connection failed");
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    self.health_tx
                        .send_modify(|health| health.last_error = Some(e.to_string()));
                    warn!(%e, "SC failover connection failed");
                }
            }
        }
        warn!(
            max_retries = self.config.max_retries,
            "SC reconnection: max retries exhausted, giving up"
        );
        let mut c = self.conn.lock().await;
        c.state = ScConnectionState::Disconnected;
        self.state_tx.send_replace(c.state);
        None
    }

    async fn publish(&self, ws: &Arc<W>, probe: &Arc<Mutex<ScConnection>>) {
        publish_connected_ws(
            self.conn,
            self.active_ws,
            ws,
            probe,
            self.state_tx,
            self.effective_max_apdu_length,
        )
        .await;
    }
}
