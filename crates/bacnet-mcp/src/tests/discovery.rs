//! Discovery's reach, network-paced Who-Is, cancellation, and an
//! Object_List read across several calls.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::Instant;

use super::{LAB, Lab};
use crate::Policy;
use crate::policy::{Caps, Rates};

/// The refusal reason of an error result, or `None` for a success.
fn reason(result: &Value) -> Option<String> {
    (result["isError"] == json!(true)).then(|| {
        result["structuredContent"]["reason"]
            .as_str()
            .unwrap_or("")
            .to_string()
    })
}

/// Without broad discovery a Who-Is must be narrow: a range of at most
/// 1000 instances. A near-full range is as broad as none.
#[tokio::test(start_paused = true)]
async fn a_wide_range_is_broad() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    for range in ["1-4194303", "0-4194302", "1000-2000"] {
        let result = lab
            .agent
            .call("bacnet_discover", json!({ "range": range, "wait_s": 1 }))
            .await;
        assert_eq!(
            reason(&result).as_deref(),
            Some("broad-discovery"),
            "{range}: {result}"
        );
    }
    assert!(lab.sends().is_empty(), "nothing was sent");
    let narrow = lab
        .agent
        .call(
            "bacnet_discover",
            json!({ "range": "1000-1999", "wait_s": 1 }),
        )
        .await;
    assert_eq!(reason(&narrow), None, "{narrow}");
    // A directed Who-Is is answered by one station only.
    let directed = lab
        .agent
        .call(
            "bacnet_discover",
            json!({ "scope": "directed", "address": "10.0.0.1", "wait_s": 1 }),
        )
        .await;
    assert_ne!(
        reason(&directed).as_deref(),
        Some("broad-discovery"),
        "{directed}"
    );
    lab.stop().await;
}

/// When the policy names its devices, a Who-Is asks only inside them,
/// broad discovery or not: a range outside, or none, is refused unsent.
#[tokio::test(start_paused = true)]
async fn a_who_is_stays_inside_the_policys_devices() {
    let policy = Policy {
        devices: Some(vec![(1000, 1300)]),
        broad_discovery: true,
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize().await;
    for args in [
        json!({ "range": "1000-1301" }),
        json!({ "range": "1-4194303" }),
        json!({}),
        json!({ "scope": "directed", "address": "10.0.0.1" }),
    ] {
        let result = lab.agent.call("bacnet_discover", args.clone()).await;
        assert_eq!(
            reason(&result).as_deref(),
            Some("device-not-allowed"),
            "{args}: {result}"
        );
    }
    assert!(lab.sends().is_empty(), "nothing was sent");
    let inside = lab
        .agent
        .call(
            "bacnet_discover",
            json!({ "range": "1200-1210", "wait_s": 1 }),
        )
        .await;
    assert_eq!(reason(&inside), None, "{inside}");
    let found: Vec<&Value> = inside["structuredContent"]["devices"]
        .as_array()
        .unwrap()
        .iter()
        .collect();
    assert!(
        found.iter().any(|d| d["instance"] == json!(LAB)),
        "{inside}"
    );
    lab.stop().await;
}

/// Broad Who-Is calls made at once are still one a minute.
#[tokio::test(start_paused = true)]
async fn parallel_broad_calls_are_one_a_minute() {
    let policy = Policy {
        broad_discovery: true,
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize().await;
    let mut ids = Vec::new();
    for _ in 0..4 {
        let call = json!({ "name": "bacnet_discover", "arguments": { "wait_s": 1 } });
        ids.push(lab.agent.post("tools/call", call).await);
    }
    let mut reasons = Vec::new();
    while reasons.len() < ids.len() {
        let reply = lab.agent.receive().await;
        if ids.contains(&reply["id"].as_u64().unwrap_or(0)) {
            reasons.push(reason(&reply["result"]));
        }
    }
    assert_eq!(
        reasons.iter().filter(|r| r.is_none()).count(),
        1,
        "{reasons:?}"
    );
    assert_eq!(
        reasons
            .iter()
            .filter(|r| r.as_deref() == Some("rate-limited"))
            .count(),
        3,
        "{reasons:?}"
    );
    assert_eq!(lab.sends().len(), 1, "one Who-Is went");
    lab.stop().await;
}

/// A network outside 1 to 65534 is refused before anything is charged.
#[tokio::test(start_paused = true)]
async fn a_network_number_is_checked_first() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    for network in [0, 65535, 70000] {
        let args = json!({ "scope": "network", "network": network, "range": "1-5" });
        let result = lab.agent.call("bacnet_discover", args).await;
        let body = &result["structuredContent"];
        assert_eq!(body["field"], json!("network"), "{network}: {result}");
        assert_eq!(body["cost"]["requests"], json!(0));
    }
    assert!(lab.sends().is_empty());
    lab.stop().await;
}

/// A Who-Is to a network the policy marks slow takes that network's slot,
/// so it is paced with the network's other requests.
#[tokio::test(start_paused = true)]
async fn a_network_who_is_is_paced_by_its_network() {
    let policy = Policy {
        slow_networks: [2001].into(),
        rates: Rates {
            global: 100,
            principal: 1000,
            burst: 1000,
            ..Rates::default()
        },
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize().await;
    let mut ids = Vec::new();
    for _ in 0..4 {
        let args = json!({ "scope": "network", "network": 2001, "range": "1-5", "wait_s": 1 });
        let call = json!({ "name": "bacnet_discover", "arguments": args });
        ids.push(lab.agent.post("tools/call", call).await);
    }
    let mut answered = 0;
    while answered < ids.len() {
        let reply = lab.agent.receive().await;
        if ids.contains(&reply["id"].as_u64().unwrap_or(0)) {
            answered += 1;
        }
    }
    let mut sends: Vec<Instant> = lab.sends().iter().map(|(at, _)| *at).collect();
    sends.sort();
    assert_eq!(sends.len(), 4);
    for pair in sends.windows(2) {
        assert!(pair[1] - pair[0] >= Duration::from_millis(500), "{sends:?}");
    }
    lab.stop().await;
}

/// A cancelled call stops: the requests still to go are never sent.
#[tokio::test(start_paused = true)]
async fn a_cancelled_call_stops_sending() {
    let mut lab = Lab::start_with(Policy::default(), Duration::from_millis(500)).await;
    lab.agent.initialize().await;
    let reads: Vec<Value> = (0..10).map(|_| json!({ "object": "ai:1" })).collect();
    let args = json!({ "device": "1201", "reads": reads });
    let id = lab
        .agent
        .post(
            "tools/call",
            json!({ "name": "bacnet_read", "arguments": args }),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    lab.agent
        .send(
            json!({ "jsonrpc": "2.0", "method": "notifications/cancelled",
                      "params": { "requestId": id, "reason": "test" } }),
        )
        .await;
    tokio::time::sleep(Duration::from_secs(20)).await;
    let sent = lab.sends().len();
    assert!(sent <= 3, "{sent} requests went after the cancel");
    lab.stop().await;
}

/// A device that can only send its Object_List element by element, with
/// more elements than one call's budget reads: each call goes on where
/// the last stopped, and every element is read once.
#[tokio::test(start_paused = true)]
async fn a_long_indexed_object_list_resumes() {
    let policy = Policy {
        rates: Rates {
            global: 1000,
            network: 1000,
            principal: 1000,
            burst: 1000,
            ..Rates::default()
        },
        caps: Caps {
            budget: Duration::from_secs(2),
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = Lab::start_lab(policy, Duration::from_millis(50), 150, true).await;
    lab.agent.initialize().await;
    let mut cursor = Value::Null;
    let mut calls = 0;
    let listed = loop {
        calls += 1;
        let mut args = json!({ "device": "1201", "types": ["analog-input"] });
        if !cursor.is_null() {
            args["cursor"] = cursor.clone();
        }
        let result = lab.agent.call("bacnet_list_objects", args).await;
        assert_eq!(result["isError"], json!(false), "{result}");
        let body = result["structuredContent"].clone();
        if body["list"]["complete"] == json!(true) {
            break body;
        }
        assert_eq!(body["list"]["indexed"], json!(true), "{body}");
        cursor = body["next_cursor"].clone();
        assert!(calls < 20, "the list never finishes");
    };
    assert!(calls >= 3, "{calls} calls");
    // The lab's own two analog inputs and the 150 more.
    assert_eq!(listed["total"], json!(152), "{listed}");
    // One whole read, one count, then each of the 154 elements once.
    let sent = lab.sends().len();
    assert_eq!(sent, 2 + 154, "{sent} requests");
    lab.stop().await;
}
