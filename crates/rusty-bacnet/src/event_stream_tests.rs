use super::*;
use std::ffi::CStr;
use std::time::Duration;

use pyo3::types::PyDict;

/// The next item, which must come from the channel.
async fn fresh<T: Clone + Send + 'static>(stream: &LossyReceiver<T>) -> Option<T> {
    stream.next().await.map(|next| match next {
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
        tokio::time::timeout(Duration::from_millis(20), stream.next())
            .await
            .is_err()
    );
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
    let pending = tokio::spawn(stream.next());
    tokio::task::yield_now().await;
    drop(tx);
    let ended = tokio::time::timeout(Duration::from_secs(5), pending).await;
    assert!(ended.unwrap().unwrap().is_none());
}

/// A Python iterator over a `u32` channel the test feeds from Python.
#[pyclass(frozen)]
struct Probe {
    tx: broadcast::Sender<u32>,
    stream: LossyReceiver<u32>,
}

#[pymethods]
impl Probe {
    fn send(&self, item: u32) {
        self.tx.send(item).unwrap();
    }

    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let next = self.stream.next();
        crate::py_async::future_into_py_or_put_back(
            py,
            async move { next.await.ok_or_else(|| ended("probe")) },
            self.stream.put_back(),
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
            tx,
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

/// Cancelled before the binding thread could take the GIL to deliver: it
/// finds the future done and puts the item back. A second item put back
/// while the slot is full counts as missed.
#[test]
fn an_item_cancelled_before_delivery_comes_back_and_a_second_is_counted() {
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
        # Holds the GIL, so both items wait to be delivered.
        until = time.perf_counter() + 0.3
        while time.perf_counter() < until:
            pass
        first.cancel()
        second.cancel()
    finally:
        sys.setswitchinterval(interval)
    # Both deliveries take the GIL and find their futures cancelled.
    for _ in range(500):
        if probe.missed:
            break
        await asyncio.sleep(0.01)
    item = await asyncio.wait_for(probe.next(), 5)
    assert item in (1, 2), item
    assert probe.missed == 1
    probe.send(3)
    assert await asyncio.wait_for(probe.next(), 5) == 3
asyncio.run(main())
"#);
}
