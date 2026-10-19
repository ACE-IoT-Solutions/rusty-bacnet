use super::*;
use std::time::Duration;

#[tokio::test]
async fn a_lagging_receiver_counts_the_gap_and_never_holds_up_the_sender() {
    let (tx, rx) = broadcast::channel(4);
    let stream = LossyReceiver::new(rx);
    // Nobody reads while ten items go out: each send returns at once.
    for item in 0..10u32 {
        tx.send(item).unwrap();
    }
    assert_eq!(stream.missed(), 0);
    // The six oldest are gone; the stream picks up at the oldest held.
    assert_eq!(stream.next().await, Some(6));
    assert_eq!(stream.missed(), 6);
    for expected in 7..10 {
        assert_eq!(stream.next().await, Some(expected));
    }
    assert_eq!(stream.missed(), 6);
}

#[tokio::test]
async fn a_cancelled_next_loses_nothing_and_the_end_follows_the_last_item() {
    let (tx, rx) = broadcast::channel(4);
    let stream = LossyReceiver::new(rx);
    // A next() given up before anything came, as a cancelled __anext__.
    assert!(
        tokio::time::timeout(Duration::from_millis(20), stream.next())
            .await
            .is_err()
    );
    tx.send(1u32).unwrap();
    tx.send(2).unwrap();
    drop(tx);
    assert_eq!(stream.next().await, Some(1));
    assert_eq!(stream.next().await, Some(2));
    assert_eq!(stream.next().await, None);
    assert_eq!(stream.next().await, None);
}

#[tokio::test]
async fn a_pending_next_ends_when_the_last_sender_drops() {
    let (tx, rx) = broadcast::channel::<u32>(4);
    let stream = LossyReceiver::new(rx);
    let pending = tokio::spawn(stream.next());
    tokio::task::yield_now().await;
    drop(tx);
    let ended = tokio::time::timeout(Duration::from_secs(5), pending).await;
    assert_eq!(ended.unwrap().unwrap(), None);
}
