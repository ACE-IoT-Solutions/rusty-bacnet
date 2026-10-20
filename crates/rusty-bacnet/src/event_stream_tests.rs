use super::*;
use std::ffi::CStr;
use std::time::Duration;

use pyo3::types::PyDict;

/// The next item, which must come from the channel.
async fn fresh<T: Clone + Send + 'static>(stream: &LossyReceiver<T>) -> Option<T> {
    stream.next().0.await.map(|next| match next {
        Next::Fresh(item) => item,
        Next::Again(_) => panic!("nothing was put back"),
    })
}

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
    assert_eq!(fresh(&stream).await, Some(6));
    assert_eq!(stream.missed(), 6);
    for expected in 7..10 {
        assert_eq!(fresh(&stream).await, Some(expected));
    }
    assert_eq!(stream.missed(), 6);
}

#[tokio::test]
async fn a_cancelled_next_loses_nothing_and_the_end_follows_the_last_item() {
    let (tx, rx) = broadcast::channel(4);
    let stream = LossyReceiver::new(rx);
    // A next() given up before anything came, as a cancelled __anext__.
    assert!(
        tokio::time::timeout(Duration::from_millis(20), stream.next().0)
            .await
            .is_err()
    );
    // Given up before it armed, it left nothing in flight.
    assert_eq!(stream.slot.in_flight.load(Ordering::Acquire), 0);
    tx.send(1u32).unwrap();
    tx.send(2).unwrap();
    drop(tx);
    assert_eq!(fresh(&stream).await, Some(1));
    assert_eq!(fresh(&stream).await, Some(2));
    assert_eq!(fresh(&stream).await, None);
    assert_eq!(fresh(&stream).await, None);
}

#[tokio::test]
async fn a_pending_next_ends_when_the_last_sender_drops() {
    let (tx, rx) = broadcast::channel::<u32>(4);
    let stream = LossyReceiver::new(rx);
    let pending = tokio::spawn(stream.next().0);
    tokio::task::yield_now().await;
    drop(tx);
    let ended = tokio::time::timeout(Duration::from_secs(5), pending).await;
    assert!(ended.unwrap().unwrap().is_none());
}

/// An item handed out and then dropped on its way, its delivery never
/// settled by hand, still lets the next `next()` through.
#[tokio::test]
async fn a_delivery_dropped_unsettled_leaves_nothing_in_flight() {
    let (tx, rx) = broadcast::channel(4);
    let stream = LossyReceiver::new(rx);
    tx.send(1u32).unwrap();
    tx.send(2).unwrap();
    let (next, hook) = stream.next();
    assert!(matches!(next.await, Some(Next::Fresh(1))));
    assert_eq!(stream.slot.in_flight.load(Ordering::Acquire), 1);
    drop(hook);
    assert_eq!(stream.slot.in_flight.load(Ordering::Acquire), 0);
    let second = tokio::time::timeout(Duration::from_secs(5), fresh(&stream)).await;
    assert_eq!(second.unwrap(), Some(2));
}

/// While a delivery is in flight the next `next()` waits, even with the
/// next item already in the channel; the put-back item then comes first.
#[tokio::test]
async fn a_put_back_item_comes_before_a_later_one_already_queued() {
    Python::initialize();
    let (tx, rx) = broadcast::channel(4);
    let stream = LossyReceiver::new(rx);
    tx.send(1u32).unwrap();
    tx.send(2).unwrap();
    let (first, first_hook) = stream.next();
    assert!(matches!(first.await, Some(Next::Fresh(1))));
    let (second, second_hook) = stream.next();
    let second = tokio::spawn(second);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!second.is_finished(), "read past an item still in flight");
    Python::attach(|py| {
        first_hook.put_back(py, 1u32.into_py_any(py).unwrap());
        first_hook.settled();
    });
    let again = tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .unwrap()
        .unwrap();
    match again {
        Some(Next::Again(object)) => {
            Python::attach(|py| assert_eq!(object.extract::<u32>(py).unwrap(), 1));
        }
        _ => panic!("expected the put-back item"),
    }
    // Delivered: until its delivery settles, the next read waits behind it.
    drop(second_hook);
    assert_eq!(fresh(&stream).await, Some(2));
    assert_eq!(stream.missed(), 0);
}

/// The slot keeps the older item and counts a newer one as missed.
#[test]
fn a_full_slot_keeps_the_older_item_and_counts_the_newer() {
    Python::initialize();
    Python::attach(|py| {
        let slot = Redelivery::default();
        slot.put_back(1u32.into_py_any(py).unwrap());
        slot.put_back(2u32.into_py_any(py).unwrap());
        assert_eq!(slot.missed.load(Ordering::Relaxed), 1);
        assert_eq!(slot.take().unwrap().extract::<u32>(py).unwrap(), 1);
        assert!(slot.take().is_none());
    });
}

/// A Python iterator over a `u32` channel the test feeds from Python.
#[pyclass(frozen)]
struct Probe {
    tx: StdMutex<Option<broadcast::Sender<u32>>>,
    stream: LossyReceiver<u32>,
}

#[pymethods]
impl Probe {
    fn send(&self, item: u32) {
        let tx = self.tx.lock().unwrap();
        tx.as_ref().expect("probe closed").send(item).unwrap();
    }

    /// Drop the sender, so the stream ends after what it holds.
    fn close(&self) {
        self.tx.lock().unwrap().take();
    }

    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let (next, put_back) = self.stream.next();
        crate::py_async::future_into_py_or_put_back(
            py,
            async move { next.await.ok_or_else(|| ended("probe")) },
            put_back,
        )
    }

    #[getter]
    fn missed(&self) -> u64 {
        self.stream.missed()
    }
}

fn run(code: &CStr) {
    Python::initialize();
    Python::attach(|py| {
        let scope = PyDict::new(py);
        let (tx, rx) = broadcast::channel(16);
        let probe = Probe {
            tx: StdMutex::new(Some(tx)),
            stream: LossyReceiver::new(rx),
        };
        scope
            .set_item("probe", Py::new(py, probe).unwrap())
            .unwrap();
        if let Err(err) = py.run(code, Some(&scope), None) {
            err.print(py);
            panic!("Python check failed: {err}");
        }
    });
}

/// Cancelled while its `set_result` callback waits on the event loop: the
/// callback finds the future done and puts the item back. A `next()` that
/// was already waiting on the channel wakes for it.
#[test]
fn an_item_cancelled_on_the_loop_comes_back_first() {
    run(cr#"
import asyncio, time
async def main():
    first = probe.next()
    await asyncio.sleep(0)
    probe.send(7)
    # Blocks the loop with the GIL released: the item arrives and its
    # set_result callback is queued behind this.
    time.sleep(0.3)
    first.cancel()
    # Waits on the channel before the queued callback runs.
    second = probe.next()
    assert await asyncio.wait_for(second, 5) == 7
    assert probe.missed == 0
    probe.send(8)
    assert await asyncio.wait_for(probe.next(), 5) == 8
asyncio.run(main())
"#);
}

/// A burst of two with the first read cancelled after its task returned,
/// while its `set_result` callback waits on the loop: the second read,
/// already waiting, gets the first item back, then the second.
#[test]
fn a_cancelled_read_in_a_burst_keeps_the_order() {
    run(cr#"
import asyncio, time
async def main():
    first, second = probe.next(), probe.next()
    await asyncio.sleep(0)
    probe.send(1)
    probe.send(2)
    # Blocks the loop with the GIL released: the first read's task returns
    # 1 and its set_result callback is queued behind this.
    time.sleep(0.3)
    first.cancel()
    assert await asyncio.wait_for(second, 5) == 1
    assert await asyncio.wait_for(probe.next(), 5) == 2
    assert probe.missed == 0
asyncio.run(main())
"#);
}

/// The same, with the sender gone while the first item is on its way back:
/// the item still comes before the end of the stream.
#[test]
fn a_cancelled_read_comes_back_before_the_end() {
    run(cr#"
import asyncio, time
async def main():
    first = probe.next()
    await asyncio.sleep(0)
    probe.send(1)
    time.sleep(0.3)
    first.cancel()
    probe.close()
    second = probe.next()
    assert await asyncio.wait_for(second, 5) == 1
    try:
        await asyncio.wait_for(probe.next(), 5)
    except StopAsyncIteration:
        pass
    else:
        raise AssertionError("the stream should have ended")
asyncio.run(main())
"#);
}

/// Cancelled before the binding thread could take the GIL to deliver: the
/// first read finds its future done and puts its item back. The second
/// read waited behind it, so nothing is lost and the order holds.
#[test]
fn reads_cancelled_before_delivery_lose_nothing() {
    run(cr#"
import asyncio, sys, time
async def main():
    interval = sys.getswitchinterval()
    first, second = probe.next(), probe.next()
    await asyncio.sleep(0)
    sys.setswitchinterval(30)
    try:
        probe.send(1)
        probe.send(2)
        # Holds the GIL, so the first item waits to be delivered.
        until = time.perf_counter() + 0.3
        while time.perf_counter() < until:
            pass
        first.cancel()
        second.cancel()
    finally:
        sys.setswitchinterval(interval)
    assert await asyncio.wait_for(probe.next(), 5) == 1
    assert await asyncio.wait_for(probe.next(), 5) == 2
    assert probe.missed == 0
    probe.send(3)
    assert await asyncio.wait_for(probe.next(), 5) == 3
asyncio.run(main())
"#);
}
