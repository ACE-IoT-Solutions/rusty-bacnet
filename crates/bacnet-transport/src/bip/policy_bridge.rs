//! Narrowing-only BVLL policy contract.
//!
//! Ingress must run its existing cheap structural, source, quota, and hard
//! safety gates first. This policy can then continue, silently drop, or return
//! the function-specific NAK; it has no verdict capable of widening admission
//! or bypassing forwarding budgets.

use bacnet_types::enums::{BvlcFunction, BvlcResultCode};

/// Maximum payload prefix exposed to an extension policy.
pub const POLICY_PAYLOAD_PREFIX_MAX: usize = 256;

/// Owned bounded context suitable for transfer to an isolated policy worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BvllPolicyContext {
    pub function: BvlcFunction,
    pub source_ip: [u8; 4],
    pub source_port: u16,
    pub payload_len: usize,
    pub payload_prefix: Vec<u8>,
}

impl BvllPolicyContext {
    pub fn new(
        function: BvlcFunction,
        source_ip: [u8; 4],
        source_port: u16,
        payload: &[u8],
    ) -> Self {
        Self {
            function,
            source_ip,
            source_port,
            payload_len: payload.len(),
            payload_prefix: payload[..payload.len().min(POLICY_PAYLOAD_PREFIX_MAX)].to_vec(),
        }
    }
}

/// A policy can only narrow the result of native hard-gate admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BvllPolicyVerdict {
    Continue,
    Drop,
    Reject(BvlcResultCode),
}

/// Thread-safe native policy. Language-runtime bridges should invoke user code
/// on a bounded worker queue and return a fail-closed result on timeout,
/// overflow, panic, or invalid output.
pub trait BvllPolicy: Send + Sync + 'static {
    fn evaluate(&self, context: &BvllPolicyContext) -> BvllPolicyVerdict;
}

/// Result after preserving native hard-gate precedence and validating that a
/// rejection code belongs to the incoming function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluatedBvllPolicy {
    Continue,
    Drop,
    Reject(BvlcResultCode),
}

pub fn evaluate_narrowing_policy(
    hard_gate_admitted: bool,
    policy: Option<&dyn BvllPolicy>,
    context: &BvllPolicyContext,
) -> EvaluatedBvllPolicy {
    if !hard_gate_admitted {
        return EvaluatedBvllPolicy::Drop;
    }
    let verdict = policy.map(|policy| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| policy.evaluate(context)))
            .unwrap_or(BvllPolicyVerdict::Drop)
    });
    match verdict {
        None | Some(BvllPolicyVerdict::Continue) => EvaluatedBvllPolicy::Continue,
        Some(BvllPolicyVerdict::Drop) => EvaluatedBvllPolicy::Drop,
        Some(BvllPolicyVerdict::Reject(code)) if valid_reject(context.function, code) => {
            EvaluatedBvllPolicy::Reject(code)
        }
        Some(BvllPolicyVerdict::Reject(_)) => EvaluatedBvllPolicy::Drop,
    }
}

fn valid_reject(function: BvlcFunction, code: BvlcResultCode) -> bool {
    let expected = if function == BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE {
        Some(BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK)
    } else if function == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE {
        Some(BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK)
    } else if function == BvlcFunction::REGISTER_FOREIGN_DEVICE {
        Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK)
    } else if function == BvlcFunction::READ_FOREIGN_DEVICE_TABLE {
        Some(BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK)
    } else if function == BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY {
        Some(BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK)
    } else if function == BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK {
        Some(BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK)
    } else {
        None
    };
    expected == Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crate::bvll::{decode_bip_mac, encode_bvll};
    use crate::port::TransportPort;

    use super::super::BipTransport;

    struct CountingPolicy(AtomicUsize, BvllPolicyVerdict);

    impl BvllPolicy for CountingPolicy {
        fn evaluate(&self, _context: &BvllPolicyContext) -> BvllPolicyVerdict {
            self.0.fetch_add(1, Ordering::Relaxed);
            self.1
        }
    }

    #[test]
    fn hard_gate_denial_has_precedence_and_skips_extension() {
        let policy = CountingPolicy(AtomicUsize::new(0), BvllPolicyVerdict::Continue);
        let context = BvllPolicyContext::new(
            BvlcFunction::REGISTER_FOREIGN_DEVICE,
            [192, 0, 2, 1],
            47808,
            &[0; 400],
        );
        assert_eq!(context.payload_prefix.len(), POLICY_PAYLOAD_PREFIX_MAX);
        assert_eq!(
            evaluate_narrowing_policy(false, Some(&policy), &context),
            EvaluatedBvllPolicy::Drop
        );
        assert_eq!(policy.0.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn invalid_cross_function_rejection_fails_closed() {
        let policy = CountingPolicy(
            AtomicUsize::new(0),
            BvllPolicyVerdict::Reject(BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK),
        );
        let context = BvllPolicyContext::new(
            BvlcFunction::REGISTER_FOREIGN_DEVICE,
            [192, 0, 2, 1],
            47808,
            &[],
        );
        assert_eq!(
            evaluate_narrowing_policy(true, Some(&policy), &context),
            EvaluatedBvllPolicy::Drop
        );
    }

    #[tokio::test]
    async fn installed_drop_policy_runs_before_local_delivery() {
        let policy = Arc::new(CountingPolicy(AtomicUsize::new(0), BvllPolicyVerdict::Drop));
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        transport.set_bvll_policy(policy.clone()).unwrap();
        let mut received = transport.start().await.unwrap();
        let (ip, port) = decode_bip_mac(transport.local_mac()).unwrap();
        let sender = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let mut frame = BytesMut::new();
        encode_bvll(&mut frame, BvlcFunction::ORIGINAL_UNICAST_NPDU, &[1, 2, 3]).unwrap();
        sender
            .send_to(&frame, SocketAddrV4::new(Ipv4Addr::from(ip), port))
            .await
            .unwrap();

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), received.recv())
                .await
                .is_err()
        );
        assert_eq!(policy.0.load(Ordering::Relaxed), 1);
        transport.stop().await.unwrap();
    }
}
