use super::super::config::PortConfig;
use super::*;
use bacnet_transport::bip::BipMode;
use tokio::sync::oneshot;

async fn started() -> (Lifecycle<NativeSession>, u64) {
    let owner = Lifecycle::new();
    let generation = owner
        .start(false, || {
            NativeSession::new(vec![PortConfig {
                network_number: 1,
                interface: std::net::Ipv4Addr::LOCALHOST,
                port: 0,
                broadcast: std::net::Ipv4Addr::LOCALHOST,
                mode: BipMode::Normal,
            }])
        })
        .await
        .unwrap();
    (owner, generation)
}

#[tokio::test]
async fn lost_publication_at_each_bridge_boundary_joins_its_generation() {
    Python::initialize();
    for via in [Via::BeforeDelivery, Via::OnLoop, Via::ClosedLoop] {
        let (owner, generation) = started().await;
        let publication = Publication::new(owner.clone());
        publication.arm(generation);
        let (tx, done) = oneshot::channel();
        *publication.completed.lock().unwrap() = Some(tx);
        Python::attach(|py| publication.put_back(py, py.None(), via));
        publication.settled();
        done.await.unwrap();
        assert!(owner.inspect(|_| ()).await.is_none());
        owner.close().await.unwrap();
    }
}

#[tokio::test]
async fn unsettled_drop_cleans_but_a_delivered_result_keeps_ownership() {
    let (owner, generation) = started().await;
    let publication = Publication::new(owner.clone());
    publication.arm(generation);
    let (tx, done) = oneshot::channel();
    *publication.completed.lock().unwrap() = Some(tx);
    drop(publication);
    done.await.unwrap();
    assert!(owner.inspect(|_| ()).await.is_none());
    let (owner, generation) = started().await;
    let publication = Publication::new(owner.clone());
    publication.arm(generation);
    let (tx, done) = oneshot::channel();
    *publication.completed.lock().unwrap() = Some(tx);
    publication.settled();
    drop(publication);
    assert!(done.await.is_err());
    assert!(owner.inspect(|_| ()).await.is_some());
    owner.close().await.unwrap();
}
