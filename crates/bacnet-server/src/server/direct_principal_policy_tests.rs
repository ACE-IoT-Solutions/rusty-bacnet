//! `ServerPolicy` over real direct TLS (#524): the principal is the verified
//! leaf, so a policy tells two CA-valid peers apart, and a denied write
//! changes nothing.
use super::*;
use crate::mutation::{MutationAuthorizationContext, MutationServiceCounters};
use crate::ServerPolicy;
use bacnet_transport::port::ScPrincipal;

/// Allows writes from one pinned principal; records what each call saw.
#[derive(Default)]
struct PinnedWriter {
    pinned: StdMutex<Option<ScPrincipal>>,
    seen: StdMutex<Vec<(Option<ScPrincipal>, String)>>,
}

impl ServerPolicy for Arc<PinnedWriter> {
    fn authorize_mutation(&self, context: &MutationAuthorizationContext) -> bool {
        let principal = context.principal();
        self.seen
            .lock()
            .unwrap()
            .push((principal, format!("{context:?}")));
        principal.is_some() && principal == *self.pinned.lock().unwrap()
    }
}

fn write(text: &str, invoke: u8) -> Apdu {
    confirmed(
        ConfirmedServiceChoice::WRITE_PROPERTY,
        invoke,
        write_payload(text),
    )
}

async fn present_value(f: &Fixture) -> PropertyValue {
    f.server
        .db
        .read()
        .await
        .get(&csv_oid())
        .unwrap()
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap()
}

#[tokio::test]
async fn server_policy_tells_direct_leaf_principals_apart() {
    let ca = TestCa::new();
    let (a_tls, b_tls) = (ca.tls("a"), ca.tls("b"));
    let policy = Arc::new(PinnedWriter::default());
    let config = ServerConfig {
        policy: Some(Arc::new(policy.clone())),
        ..Default::default()
    };
    let mut f = Fixture::new(&ca, config, database(Arc::new(AtomicUsize::new(0)))).await;

    // A's principal is the SHA-256 of its verified leaf, with no incarnation.
    let mut a = f.peer(a_tls.clone()).await;
    let from_a = f.capture(&mut a, &write("from A", 1)).await;
    let a_identity = from_a.provenance.direct_sc_identity().unwrap();
    let a_principal = from_a.provenance.principal().unwrap();
    assert_eq!(a_principal.leaf_sha256(), a_identity.leaf_sha256());
    *policy.pinned.lock().unwrap() = Some(a_principal);
    f.feed(from_a).await;
    assert!(matches!(f.response().await, Apdu::SimpleAck(_)));
    assert_eq!(
        present_value(&f).await,
        PropertyValue::CharacterString("from A".into())
    );

    // B, signed by the same CA and claiming the same VMAC and UUID, is a
    // different principal: denied, and nothing changes.
    let mut b = f.peer(b_tls).await;
    let from_b = f.capture(&mut b, &write("from B", 2)).await;
    let b_principal = from_b.provenance.principal().unwrap();
    assert_ne!(b_principal, a_principal);
    f.feed(from_b).await;
    assert!(denied(&f.response().await));
    assert_eq!(
        present_value(&f).await,
        PropertyValue::CharacterString("from A".into())
    );

    // A reconnecting gets a new incarnation but keeps its principal.
    let mut again = f.peer(a_tls).await;
    let from_again = f.capture(&mut again, &write("A again", 3)).await;
    let again_identity = from_again.provenance.direct_sc_identity().unwrap();
    assert_ne!(again_identity.incarnation(), a_identity.incarnation());
    assert_eq!(from_again.provenance.principal(), Some(a_principal));
    f.feed(from_again).await;
    assert!(matches!(f.response().await, Apdu::SimpleAck(_)));
    assert_eq!(
        present_value(&f).await,
        PropertyValue::CharacterString("A again".into())
    );

    let seen = policy.seen.lock().unwrap().clone();
    let principals: Vec<_> = seen.iter().map(|(principal, _)| *principal).collect();
    assert_eq!(
        principals,
        [Some(a_principal), Some(b_principal), Some(a_principal)]
    );
    for (_, debug) in &seen {
        // Debug says a principal is present, never its fingerprint.
        assert!(debug.contains("principal: true"));
        assert!(!debug.contains(&a_principal.to_string()[..8]));
        assert!(!debug.contains("from "));
    }
    assert_eq!(
        f.server.mutation_decision_counters().write_property,
        MutationServiceCounters {
            allow_total: 2,
            deny_total: 1,
            policy_deny_total: 0,
        }
    );
    f.stop().await;
}
