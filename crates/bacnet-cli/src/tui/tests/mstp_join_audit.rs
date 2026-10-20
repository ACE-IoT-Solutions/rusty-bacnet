//! A confirmed MS/TP trunk join on a profile session goes into the
//! profile's audit log (#1862); a passive session, which joins nothing,
//! records none. (A declined join sends no Connect at all; see
//! `mstp_screen::an_mstp_profile_connects_only_after_the_join_is_confirmed`.)

use std::path::Path;

use serde_json::Value;
use tokio::sync::mpsc;

use super::connections::event;
use super::*;
use crate::tui::profile::service::{Request, Service};
use crate::tui::profile::store::Store;
use crate::tui::profile::{Overrides, Profile};
use crate::tui::safety::audit::MSTP_JOIN;
use crate::tui::worker::session::SessionOptions;
use crate::tui::worker::{coordinator, EventSink};

/// A port no host has, so the session's open fails and nothing is sent.
const PORT: &str = "/nonexistent/ttyBACnet-1862";

fn trunk() -> Profile {
    let mut p = Profile::mstp_default();
    let m = p.mstp_mut();
    m.port = PORT.into();
    m.mac = 3;
    m.baud = 76_800;
    p
}

fn records(root: &Path) -> Vec<Value> {
    let path = root.join("workspaces/trunk/audit.jsonl");
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Connect profile `trunk`, as the app does once its join is confirmed,
/// and return once the coordinator has answered.
async fn join(root: &Path, options: SessionOptions) {
    let store = Store::new(root.into());
    store.save("trunk", &trunk(), true).unwrap();
    let service = Service::new(Ok(store), Overrides::default());
    let (sink, mut events) = EventSink::channel(16);
    let (commands, rx) = mpsc::channel(16);
    let task = coordinator::spawn(None, Some(service), sink, rx, options);
    let connect = Request::Connect {
        name: "trunk".into(),
        profile: trunk(),
        attest: true,
        accepted: Vec::new(),
    };
    commands.send(Command::Profile(connect)).await.unwrap();
    let reply = event(&mut events).await;
    assert!(matches!(reply, WorkerEvent::Profile(Ok(_))), "{reply:?}");
    drop((commands, events));
    task.await.unwrap();
}

#[tokio::test]
async fn a_confirmed_profile_join_is_recorded_once() {
    let root = tempfile::tempdir().unwrap();
    join(root.path(), SessionOptions::default()).await;
    let records = records(root.path());
    let [record] = records.as_slice() else {
        panic!("{records:?}");
    };
    assert_eq!(record["event"], MSTP_JOIN);
    assert_eq!(record["profile"], "trunk");
    assert_eq!(record["transport"], "MSTP");
    assert_eq!(record["port"], PORT);
    assert_eq!(record["station"], 3);
    assert_eq!(record["baud"], 76_800);
    assert!(record["time"].as_str().is_some_and(|t| t.ends_with('Z')));
}

#[tokio::test]
async fn a_passive_profile_session_records_no_join() {
    let root = tempfile::tempdir().unwrap();
    let mut options = SessionOptions::default();
    options.safety.passive = true;
    join(root.path(), options).await;
    assert!(records(root.path()).is_empty());
}
