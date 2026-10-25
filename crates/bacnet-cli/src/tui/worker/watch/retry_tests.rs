//! Subscription retry scheduling and the independent cancellation obligation.

use super::*;

/// The lab device, on this network.
fn lab() -> CovTarget {
    CovTarget::local(&[10, 0, 0, 100, 0xBA, 0xC0])
}

fn failed(maybe_subscribed: bool, fatal: bool) -> Started {
    Started {
        first: None,
        name: None,
        target: None,
        outcome: Err(Refusal {
            reason: "no answer".into(),
            maybe_subscribed,
            retryable: !fatal,
            fatal,
        }),
        refused_rpm: false,
    }
}

fn retrying<'a>(sink: &'a EventSink, options: &'a WatchOptions, ambiguous: bool) -> State<'a> {
    let mut state = state_with_row(sink, options);
    state.devices.get_mut(&DEVICE).unwrap().ops.clear();
    let row = state.rows.get_mut(&1).unwrap();
    row.target = Some(lab());
    row.maybe_subscribed = ambiguous;
    state
}

fn assert_cancel(state: &State<'_>, expected: bool) {
    assert!(!state.rows.contains_key(&1));
    assert_eq!(state.cancels, usize::from(expected));
    let ops = &state.devices[&DEVICE].ops;
    assert_eq!(ops.len(), usize::from(expected));
    if expected {
        let Op::Cancel {
            row,
            point,
            target,
            managed,
        } = &ops[0]
        else {
            panic!("expected cancellation")
        };
        assert_eq!(*row, 1);
        assert_eq!(point.device, DEVICE);
        assert_eq!(*target, lab());
        assert!(managed.is_none());
    }
}

#[tokio::test(start_paused = true)]
async fn initial_timeout_schedules_one_start_after_thirty_seconds() {
    let (sink, _events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let options = WatchOptions::default();
    let mut state = retrying(&sink, &options, false);
    state.started(1, failed(false, false));
    let rate = RateCap::new(0);
    tokio::time::advance(Duration::from_secs(29)).await;
    state.tick(&rate);
    assert!(state.devices[&DEVICE].ops.is_empty());
    tokio::time::advance(Duration::from_secs(1)).await;
    state.tick(&rate);
    state.tick(&rate);
    assert_eq!(state.devices[&DEVICE].ops.len(), 1);
    assert!(matches!(state.devices[&DEVICE].ops[0], Op::Start(1)));
}

#[tokio::test(start_paused = true)]
async fn queued_retry_remove_and_shutdown_keep_prior_cancellation() {
    for shutdown in [false, true] {
        for ambiguous in [false, true] {
            let (sink, _events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
            let options = WatchOptions::default();
            let mut state = retrying(&sink, &options, ambiguous);
            let row = state.rows.get_mut(&1).unwrap();
            row.mode = Mode::Polled;
            row.resubscribe = Some((Instant::now(), Duration::from_secs(30)));
            state.devices.get_mut(&DEVICE).unwrap().busy = true;
            state.tick(&RateCap::new(0));
            assert!(matches!(state.devices[&DEVICE].ops[0], Op::Start(1)));
            if shutdown {
                state.shutdown();
            } else {
                state.remove(1);
            }
            state.tick(&RateCap::new(0));
            assert_cancel(&state, ambiguous);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn inflight_retry_remove_and_shutdown_keep_old_mac_and_cancellation() {
    for shutdown in [false, true] {
        for ambiguous in [false, true] {
            for (fatal, retryable) in [(false, true), (false, false), (true, false)] {
                let (sink, _events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
                let options = WatchOptions::default();
                let mut state = retrying(&sink, &options, ambiguous);
                if shutdown {
                    state.shutdown();
                } else {
                    state.remove(1);
                }
                assert!(state.rows[&1].removing);
                let mut outcome = failed(false, fatal);
                outcome.outcome.as_mut().err().unwrap().retryable = retryable;
                state.started(1, outcome);
                state.tick(&RateCap::new(0));
                assert_cancel(&state, ambiguous);
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn fatal_retry_failure_disposes_prior_cancellation() {
    let (sink, _events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let options = WatchOptions::default();
    let mut state = retrying(&sink, &options, true);
    state.started(1, failed(false, true));
    assert_cancel(&state, true);
}

#[tokio::test(start_paused = true)]
async fn repeated_failures_back_off_to_cap_and_permanent_refusal_stops_retry_only() {
    let (sink, _events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let options = WatchOptions::default();
    let mut state = retrying(&sink, &options, true);
    let rate = RateCap::new(0);
    for seconds in [30, 60, 120, 240, 300, 300] {
        state.started(1, failed(false, false));
        assert!(
            state.rows[&1].maybe_subscribed,
            "read-only retry retains earlier obligation"
        );
        tokio::time::advance(Duration::from_secs(seconds - 1)).await;
        state.tick(&rate);
        assert!(state.devices[&DEVICE].ops.is_empty());
        tokio::time::advance(Duration::from_secs(1)).await;
        state.tick(&rate);
        state.tick(&rate);
        let ops = &mut state.devices.get_mut(&DEVICE).unwrap().ops;
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops.pop_front(), Some(Op::Start(1))));
        state.rows.get_mut(&1).unwrap().mode = Mode::Starting;
    }
    let mut permanent = failed(false, false);
    permanent.outcome.as_mut().err().unwrap().retryable = false;
    state.started(1, permanent);
    assert!(state.rows[&1].resubscribe.is_none());
    assert!(state.rows[&1].maybe_subscribed);
    tokio::time::advance(Duration::from_secs(600)).await;
    state.tick(&rate);
    assert!(state.devices[&DEVICE].ops.is_empty());
    state.remove(1);
    assert_cancel(&state, true);
}

async fn successful_retry(shutdown: bool) {
    use super::super::retry_support::{self, Reply};
    use std::sync::atomic::Ordering;

    let (sink, _events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let options = WatchOptions::default();
    let mut state = retrying(&sink, &options, true);
    state.rows.get_mut(&1).unwrap().resubscribe = Some((Instant::now(), Duration::from_secs(300)));
    let (client, requests) = retry_support::client(Reply::Ack).await;
    let managed = Arc::clone(&client)
        .manage_cov_subscription(
            lab(),
            1,
            ai(1),
            false,
            300,
            bacnet_client::client::ManagedCOVSubscriptionOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(requests.load(Ordering::Relaxed), 1);
    if shutdown {
        state.shutdown();
    }
    state.started(
        1,
        Started {
            first: None,
            name: None,
            target: Some(lab()),
            outcome: Ok(managed),
            refused_rpm: false,
        },
    );
    if shutdown {
        assert!(state.rows.is_empty());
        assert!(!state.outbox.iter().any(|event| matches!(
            event,
            WatchEvent::Source {
                source: Source::Cov { .. },
                ..
            }
        )));
        assert_eq!(state.cancels, 1);
        let Op::Cancel {
            row,
            point,
            target,
            managed,
        } = state
            .devices
            .get_mut(&DEVICE)
            .unwrap()
            .ops
            .pop_front()
            .unwrap()
        else {
            panic!("late success was not cancelled")
        };
        assert!(managed.is_some(), "cleanup owns and stops the renewal task");
        jobs::cancel(&client, &RateCap::new(0), row, point, target, managed)
            .await
            .unwrap();
        assert_eq!(requests.load(Ordering::Relaxed), 2);
        tokio::time::advance(Duration::from_secs(600)).await;
        assert_eq!(
            requests.load(Ordering::Relaxed),
            2,
            "no renewal after cancellation"
        );
    } else {
        let row = state.rows.get_mut(&1).unwrap();
        assert!(row.resubscribe.is_none() && !row.maybe_subscribed);
        let Mode::Cov(managed) = std::mem::replace(&mut row.mode, Mode::Starting) else {
            panic!("not subscribed")
        };
        managed.stop().await;
        state.started(1, failed(false, false));
        tokio::time::advance(Duration::from_secs(30)).await;
        state.tick(&RateCap::new(0));
        assert!(
            matches!(state.devices[&DEVICE].ops.front(), Some(Op::Start(1))),
            "successful start reset the backoff"
        );
    }
    retry_support::stop(client).await;
}

#[tokio::test(start_paused = true)]
async fn successful_start_resets_retry_backoff_and_ambiguity() {
    successful_retry(false).await;
}

#[tokio::test(start_paused = true)]
async fn late_success_after_shutdown_is_stopped_and_cancelled() {
    successful_retry(true).await;
}
