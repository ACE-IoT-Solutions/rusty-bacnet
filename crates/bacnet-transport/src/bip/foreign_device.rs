use std::collections::HashMap;
use std::net::SocketAddrV4;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Mutex as StdMutex};
use std::time::{Duration, Instant};

use bacnet_types::enums::BvlcResultCode;
use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tokio::time::Instant as TokioInstant;
use tracing::warn;

use crate::bvll::{encode_bvll, BvllMessage};
use bacnet_types::enums::BvlcFunction;

use super::{decode_bvlc_result_code, BvlcResponseKind, PendingBvlcCleanup, PendingBvlcResponse};

/// Live state of a managed BACnet/IP foreign-device registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForeignDeviceRegistrationState {
    /// No conclusive response has been received yet.
    Pending,
    /// The BBMD accepted the most recent registration.
    Registered,
    /// The BBMD explicitly rejected the most recent registration.
    Rejected,
    /// The last successful registration has exceeded its TTL.
    Expired,
}

/// Point-in-time view of a managed foreign-device registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForeignDeviceRegistrationStatus {
    /// Current managed-registration state.
    pub state: ForeignDeviceRegistrationState,
    /// Most recently received BVLC result code.
    pub last_result_code: Option<BvlcResultCode>,
    /// Rounded-up whole seconds until the next renewal attempt.
    pub seconds_to_renewal: Option<u64>,
}

#[derive(Debug)]
struct RegistrationTelemetry {
    state: ForeignDeviceRegistrationState,
    last_result_code: Option<BvlcResultCode>,
    expires_at: Instant,
    next_renewal: Instant,
    ttl: Duration,
}

/// Cloneable, read-only capability for observing background registration.
#[derive(Clone, Debug)]
pub struct ForeignDeviceRegistrationHandle {
    inner: Arc<Mutex<RegistrationTelemetry>>,
}

impl ForeignDeviceRegistrationHandle {
    pub(super) fn new(ttl: u16) -> Self {
        let now = Instant::now();
        Self {
            inner: Arc::new(Mutex::new(RegistrationTelemetry {
                state: ForeignDeviceRegistrationState::Pending,
                last_result_code: None,
                expires_at: now + Duration::from_secs(u64::from(ttl)),
                next_renewal: now,
                ttl: Duration::from_secs(u64::from(ttl)),
            })),
        }
    }

    /// Return a point-in-time snapshot without network I/O.
    pub fn status(&self) -> ForeignDeviceRegistrationStatus {
        self.status_at(Instant::now())
    }

    fn status_at(&self, now: Instant) -> ForeignDeviceRegistrationStatus {
        let mut telemetry = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(
            telemetry.state,
            ForeignDeviceRegistrationState::Pending
                | ForeignDeviceRegistrationState::Registered
                | ForeignDeviceRegistrationState::Rejected
        ) && now >= telemetry.expires_at
        {
            telemetry.state = ForeignDeviceRegistrationState::Expired;
        }
        let seconds_to_renewal = matches!(
            telemetry.state,
            ForeignDeviceRegistrationState::Pending | ForeignDeviceRegistrationState::Registered
        )
        .then(|| {
            let remaining = telemetry.next_renewal.saturating_duration_since(now);
            remaining.as_secs() + u64::from(remaining.subsec_nanos() != 0)
        });
        ForeignDeviceRegistrationStatus {
            state: telemetry.state,
            last_result_code: telemetry.last_result_code,
            seconds_to_renewal,
        }
    }

    pub(super) fn record_result(
        &self,
        code: BvlcResultCode,
        now: Instant,
        renewal_interval: Duration,
    ) {
        let mut telemetry = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        telemetry.last_result_code = Some(code);
        telemetry.next_renewal = now + renewal_interval;
        if code == BvlcResultCode::SUCCESSFUL_COMPLETION {
            telemetry.state = ForeignDeviceRegistrationState::Registered;
            telemetry.expires_at = now + telemetry.ttl;
        } else {
            telemetry.state = ForeignDeviceRegistrationState::Rejected;
        }
    }

    pub(super) fn record_no_response(&self, now: Instant, renewal_interval: Duration) {
        let mut telemetry = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        telemetry.next_renewal = now + renewal_interval;
        if matches!(
            telemetry.state,
            ForeignDeviceRegistrationState::Pending
                | ForeignDeviceRegistrationState::Registered
                | ForeignDeviceRegistrationState::Rejected
        ) && now >= telemetry.expires_at
        {
            telemetry.state = ForeignDeviceRegistrationState::Expired;
        }
    }
}

pub(super) struct RegistrationWorker {
    pub socket: Arc<UdpSocket>,
    pub bbmd_addr: SocketAddrV4,
    pub ttl: u16,
    pub handle: ForeignDeviceRegistrationHandle,
    pub pending: Arc<StdMutex<Option<PendingBvlcResponse>>>,
    pub request_lock: Arc<AsyncMutex<()>>,
    pub next_request_id: Arc<AtomicU64>,
    pub quarantine: Arc<StdMutex<HashMap<([u8; 4], u16), TokioInstant>>>,
    pub response_timeout: Duration,
}

impl RegistrationWorker {
    fn renewal_interval(&self) -> Duration {
        // BACnet TTL is integral seconds, but the renewal point is exactly
        // half the advertised lifetime and may therefore be sub-second.
        Duration::from_millis((u64::from(self.ttl) * 1_000 / 2).max(1))
    }

    async fn await_quiet_request_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        let target = (self.bbmd_addr.ip().octets(), self.bbmd_addr.port());
        loop {
            let quiet_until = self
                .quarantine
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&target)
                .copied();
            if let Some(deadline) = quiet_until.filter(|deadline| *deadline > TokioInstant::now()) {
                tokio::time::sleep_until(deadline).await;
                continue;
            }
            let guard = self.request_lock.lock().await;
            let quiet_until = self
                .quarantine
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&target)
                .copied();
            if quiet_until.is_some_and(|deadline| deadline > TokioInstant::now()) {
                drop(guard);
                continue;
            }
            self.quarantine
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&target);
            return guard;
        }
    }

    async fn register_once(&self) -> Result<BvlcResultCode, ()> {
        let _request_guard = self.await_quiet_request_guard().await;
        let target = (self.bbmd_addr.ip().octets(), self.bbmd_addr.port());
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel::<BvllMessage>();
        {
            let mut pending = self
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if pending.is_some() {
                return Err(());
            }
            *pending = Some(PendingBvlcResponse {
                id: request_id,
                target,
                expected: BvlcResponseKind::RegisterForeignDeviceResult,
                tx,
            });
        }
        let mut cleanup = PendingBvlcCleanup {
            pending: Arc::clone(&self.pending),
            quarantine: Arc::clone(&self.quarantine),
            target,
            request_id,
            armed: true,
        };

        let mut frame = BytesMut::with_capacity(6);
        if encode_bvll(
            &mut frame,
            BvlcFunction::REGISTER_FOREIGN_DEVICE,
            &self.ttl.to_be_bytes(),
        )
        .is_err()
        {
            cleanup.disarm();
            return Err(());
        }
        if let Err(error) = self.socket.send_to(&frame, self.bbmd_addr).await {
            cleanup.disarm();
            warn!(%error, bbmd = %self.bbmd_addr, "Failed managed foreign-device registration send");
            return Err(());
        }

        match tokio::time::timeout(self.response_timeout, rx).await {
            Ok(Ok(message)) => {
                cleanup.disarm();
                decode_bvlc_result_code(&message).map_err(|_| ())
            }
            Ok(Err(_)) => {
                cleanup.disarm();
                Err(())
            }
            Err(_) => Err(()),
        }
    }

    pub(super) async fn run(self) {
        let renewal_interval = self.renewal_interval();
        loop {
            match self.register_once().await {
                Ok(code) => self
                    .handle
                    .record_result(code, Instant::now(), renewal_interval),
                Err(()) => self
                    .handle
                    .record_no_response(Instant::now(), renewal_interval),
            }
            tokio::time::sleep(renewal_interval).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bbmd::ForeignDevicePolicy;
    use crate::bip::{recoverable_udp_receive_error, BipTransport, ForeignDeviceConfig};
    use crate::port::TransportPort;
    use std::net::Ipv4Addr;

    #[test]
    fn udp_peer_icmp_errors_do_not_end_the_shared_receive_loop() {
        for kind in [
            std::io::ErrorKind::ConnectionRefused,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::ConnectionAborted,
        ] {
            assert!(recoverable_udp_receive_error(&std::io::Error::from(kind)));
        }
        assert!(!recoverable_udp_receive_error(&std::io::Error::from(
            std::io::ErrorKind::BrokenPipe,
        )));
    }

    #[test]
    fn transitions_reject_expire_and_recover_without_losing_result() {
        let handle = ForeignDeviceRegistrationHandle::new(2);
        let start = Instant::now();
        assert_eq!(
            handle.status_at(start).state,
            ForeignDeviceRegistrationState::Pending
        );

        handle.record_result(
            BvlcResultCode::SUCCESSFUL_COMPLETION,
            start,
            Duration::from_secs(1),
        );
        assert_eq!(
            handle.status_at(start).state,
            ForeignDeviceRegistrationState::Registered
        );

        handle.record_result(
            BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK,
            start + Duration::from_secs(1),
            Duration::from_secs(1),
        );
        assert_eq!(
            handle.status_at(start + Duration::from_secs(1)).state,
            ForeignDeviceRegistrationState::Rejected
        );

        handle.record_no_response(start + Duration::from_secs(2), Duration::from_secs(1));
        assert_eq!(
            handle.status_at(start + Duration::from_secs(2)).state,
            ForeignDeviceRegistrationState::Expired
        );

        handle.record_result(
            BvlcResultCode::SUCCESSFUL_COMPLETION,
            start + Duration::from_secs(3),
            Duration::from_secs(1),
        );
        let recovered = handle.status_at(start + Duration::from_secs(3));
        assert_eq!(recovered.state, ForeignDeviceRegistrationState::Registered);
        assert_eq!(
            recovered.last_result_code,
            Some(BvlcResultCode::SUCCESSFUL_COMPLETION)
        );
    }

    async fn wait_for_state(
        handle: &ForeignDeviceRegistrationHandle,
        expected: ForeignDeviceRegistrationState,
    ) {
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                if handle.status().state == expected {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("foreign registration did not reach {expected:?}"));
    }

    #[tokio::test]
    async fn managed_registration_expires_and_recovers_after_bbmd_restart() {
        let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::new(127, 255, 255, 255));
        bbmd.enable_bbmd(Vec::new());
        bbmd.enable_foreign_device_registration(ForeignDevicePolicy::default());
        let _rx = bbmd.start().await.unwrap();
        let bbmd_mac = bbmd.local_mac().to_vec();
        let bbmd_port = u16::from_be_bytes([bbmd_mac[4], bbmd_mac[5]]);

        let mut foreign =
            BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::new(127, 255, 255, 255));
        foreign.register_as_foreign_device(ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::LOCALHOST,
            bbmd_port,
            ttl: 1,
        });
        let handle = foreign.foreign_device_registration().unwrap();
        let _foreign_rx = foreign.start().await.unwrap();
        wait_for_state(&handle, ForeignDeviceRegistrationState::Registered).await;

        bbmd.stop().await.unwrap();
        wait_for_state(&handle, ForeignDeviceRegistrationState::Expired).await;

        let mut restarted = BipTransport::new(
            Ipv4Addr::LOCALHOST,
            bbmd_port,
            Ipv4Addr::new(127, 255, 255, 255),
        );
        restarted.enable_bbmd(Vec::new());
        restarted.enable_foreign_device_registration(ForeignDevicePolicy::default());
        let _restarted_rx = restarted.start().await.unwrap();
        wait_for_state(&handle, ForeignDeviceRegistrationState::Registered).await;

        foreign.stop().await.unwrap();
        restarted.stop().await.unwrap();
    }

    #[tokio::test]
    async fn zero_ttl_is_rejected_before_socket_or_registration_task_start() {
        let mut transport =
            BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::new(127, 255, 255, 255));
        transport.register_as_foreign_device(ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::LOCALHOST,
            bbmd_port: 47_808,
            ttl: 0,
        });
        assert!(transport.start().await.is_err());
        assert!(transport.socket.is_none());
        assert!(transport.registration_task.is_none());
    }

    // Linux treats the complete 127/8 block as loopback, which lets this test
    // exercise distinct wire source/destination addresses without host setup.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[tokio::test]
    async fn bacpypes_result_wire_from_cross_address_completes_and_renews_at_ttl_half() {
        let bbmd_ip = Ipv4Addr::new(127, 0, 0, 2);
        let foreign_ip = Ipv4Addr::new(127, 0, 0, 3);
        let bbmd = UdpSocket::bind(SocketAddrV4::new(bbmd_ip, 0))
            .await
            .unwrap();
        let bbmd_port = bbmd.local_addr().unwrap().port();
        let responder = tokio::spawn(async move {
            let mut frame = [0_u8; 64];
            let mut arrivals = Vec::new();
            for _ in 0..2 {
                let (length, sender) = bbmd.recv_from(&mut frame).await.unwrap();
                assert_eq!(&frame[..length], &[0x81, 0x05, 0x00, 0x06, 0x00, 0x04]);
                arrivals.push(Instant::now());
                // Exact Result encoding emitted by bacpypes3 0.0.102.
                bbmd.send_to(&[0x81, 0x00, 0x00, 0x06, 0x00, 0x00], sender)
                    .await
                    .unwrap();
            }
            arrivals
        });

        let mut foreign = BipTransport::new(foreign_ip, 0, Ipv4Addr::new(127, 255, 255, 255));
        foreign.register_as_foreign_device(ForeignDeviceConfig {
            bbmd_ip,
            bbmd_port,
            ttl: 4,
        });
        let handle = foreign.foreign_device_registration().unwrap();
        let _rx = foreign.start().await.unwrap();
        wait_for_state(&handle, ForeignDeviceRegistrationState::Registered).await;

        let arrivals = tokio::time::timeout(Duration::from_secs(5), responder)
            .await
            .unwrap()
            .unwrap();
        let renewal = arrivals[1].duration_since(arrivals[0]);
        assert!(
            Duration::from_millis(1_700) <= renewal && renewal <= Duration::from_millis(2_300),
            "renewal {renewal:?} was not near TTL/2"
        );
        foreign.stop().await.unwrap();
    }
}
