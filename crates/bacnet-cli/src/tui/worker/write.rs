//! The write session's worker side: it checks every request against the
//! session level and the profile's policy again, reads the pre-image,
//! sends the writes, and owns the ledger and the audit log.
//!
//! [`run`] is polled beside the session loop under `join!`, like the
//! browser and the watch list, and serves one request at a time. It does
//! not watch for the session's cancellation while a request is out: a
//! write already sent always gets its outcome into the ledger and the
//! audit log, even if the UI never hears of it. Requests still queued when
//! the session ends are recorded as not sent.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bacnet_client::client::BACnetClient;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::{Error, ErrorDetail};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::browse::limits::RateCap;
use super::browse::read::{Halt, Reader, Wanted};
use super::EventSink;
use crate::tui::message::WorkerEvent;
use crate::tui::profile::{Safety, Writes};
use crate::tui::safety::audit::{utc_now, Audit, Context};
use crate::tui::safety::ledger::{Entry, Ledger};
use crate::tui::safety::plan::{self, PRE_READ, PRIORITY_6};
use crate::tui::safety::{
    Gate, Level, Outcome, Planned, Preview, SafetyCommand, SafetyEvent, WriteSpec, MAX_MINUTES,
};

/// Why a passive session refuses anything that transmits.
pub(crate) const PASSIVE: &str = "passive session: nothing is transmitted";

/// What the write session needs from the session's settings.
#[derive(Debug, Clone)]
pub(crate) struct SafetyOptions {
    /// The profile workspace, home of the ledger and the audit log. None
    /// in a flags-only session, which therefore can't arm.
    pub(crate) workspace: Option<PathBuf>,
    /// The profile's name.
    pub(crate) profile: Option<String>,
    /// The profile's write policy.
    pub(crate) policy: Safety,
    /// `--passive`.
    pub(crate) passive: bool,
    /// `BIP`, `BIP6` or `SC`.
    pub(crate) transport: &'static str,
}

impl Default for SafetyOptions {
    fn default() -> Self {
        Self {
            workspace: None,
            profile: None,
            policy: Safety::default(),
            passive: false,
            transport: "BIP",
        }
    }
}

/// This process's run, which marks the ledger entries it writes, so the
/// next launch can tell which ones an earlier run left.
pub(crate) fn run_id() -> &'static str {
    static RUN: OnceLock<String> = OnceLock::new();
    RUN.get_or_init(|| {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        format!("{}-{millis}", std::process::id())
    })
}

/// Open the ledger alone, to list it without a client (a passive session).
pub(crate) async fn ready_offline(sink: &EventSink, options: &SafetyOptions) {
    let ledger = options
        .workspace
        .as_deref()
        .and_then(|dir| Ledger::open(dir).ok());
    tell(
        sink,
        SafetyEvent::Ready {
            policy: options.policy.clone(),
            workspace: false,
            run: run_id().into(),
            ledger: ledger.map(|l| l.entries().to_vec()).unwrap_or_default(),
        },
    )
    .await;
}

async fn tell(sink: &EventSink, event: SafetyEvent) {
    sink.deliver(WorkerEvent::Safety(event)).await;
}

/// The open ledger and audit log, and the session's arming.
struct Runner<'a> {
    options: &'a SafetyOptions,
    store: Option<(Ledger, Audit)>,
    /// Why there is no store.
    missing: String,
    armed: Option<(Instant, String)>,
}

/// Serve write session requests until `commands` closes.
pub(crate) async fn run<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    sink: &EventSink,
    mut commands: mpsc::Receiver<SafetyCommand>,
    rate: &RateCap,
    options: &SafetyOptions,
    local: &str,
) {
    let mut runner = Runner::open(options, local);
    let ledger = runner.entries();
    tell(
        sink,
        SafetyEvent::Ready {
            policy: options.policy.clone(),
            workspace: runner.store.is_some(),
            run: run_id().into(),
            ledger,
        },
    )
    .await;
    while let Some(command) = commands.recv().await {
        runner.handle(client, sink, rate, command).await;
    }
}

impl<'a> Runner<'a> {
    fn open(options: &'a SafetyOptions, local: &str) -> Self {
        let store = match &options.workspace {
            None => Err(
                "write sessions need a profile: its workspace holds the ledger and the audit log"
                    .to_string(),
            ),
            Some(dir) => Ledger::open(dir).and_then(|ledger| {
                let context = Context {
                    profile: options.profile.clone(),
                    transport: options.transport,
                    local: local.into(),
                };
                Ok((ledger, Audit::open(dir, context)?))
            }),
        };
        let (store, missing) = match store {
            Ok(store) => (Some(store), String::new()),
            Err(error) => {
                if options.workspace.is_some() {
                    tracing::error!("{error}");
                }
                (None, error)
            }
        };
        Self {
            options,
            store,
            missing,
            armed: None,
        }
    }

    fn entries(&self) -> Vec<Entry> {
        self.store
            .as_ref()
            .map(|(ledger, _)| ledger.entries().to_vec())
            .unwrap_or_default()
    }

    fn armed(&self) -> bool {
        self.armed
            .as_ref()
            .is_some_and(|(until, _)| Instant::now() < *until)
    }

    fn level(&self) -> Level {
        if self.options.passive {
            Level::Passive
        } else if self.armed() {
            Level::Armed {
                dangerous: self.options.policy.dangerous,
            }
        } else {
            Level::ReadOnly
        }
    }

    fn audit(&mut self, event: &str, mut fields: Value) {
        let level = self.level();
        if let (Some((_, reason)), Value::Object(map)) = (&self.armed, &mut fields) {
            map.entry("session_reason").or_insert(json!(reason));
        }
        if let Some((_, audit)) = &mut self.store {
            if let Err(error) = audit.record(level, event, fields) {
                tracing::error!("{error}");
            }
        }
    }

    /// Save the ledger; false (and logged) when that failed.
    fn save(&self) -> bool {
        let Some((ledger, _)) = &self.store else {
            return false;
        };
        let saved = ledger.save();
        if let Err(error) = &saved {
            tracing::error!("{error}");
        }
        saved.is_ok()
    }

    /// Why a write can't go ahead now, if it can't.
    fn refusal(&self) -> Option<String> {
        if self.options.passive {
            Some(PASSIVE.into())
        } else if self.store.is_none() {
            Some(self.missing.clone())
        } else if !self.armed() {
            Some("read-only: press W to arm a write session".into())
        } else {
            None
        }
    }

    async fn refuse(&mut self, sink: &EventSink, req: Option<u64>, event: &str, reason: String) {
        tracing::warn!("{event} refused: {reason}");
        self.audit(
            event,
            json!({ "outcome": { "result": "refused", "why": reason } }),
        );
        tell(sink, SafetyEvent::Refused { req, reason }).await;
    }

    async fn handle<T: TransportPort + 'static>(
        &mut self,
        client: &BACnetClient<T>,
        sink: &EventSink,
        rate: &RateCap,
        command: SafetyCommand,
    ) {
        match command {
            SafetyCommand::Arm { reason, minutes } => {
                let refusal = if self.options.passive {
                    Some(PASSIVE.to_string())
                } else if self.options.policy.writes == Writes::Never {
                    Some("this profile sets writes = \"never\"".into())
                } else if self.store.is_none() {
                    Some(self.missing.clone())
                } else if !(1..=MAX_MINUTES).contains(&minutes) {
                    Some(format!("a session lasts 1 to {MAX_MINUTES} minutes"))
                } else {
                    None
                };
                if let Some(why) = refusal {
                    return self.refuse(sink, None, "arm", why).await;
                }
                let until = Instant::now() + Duration::from_secs(u64::from(minutes) * 60);
                self.armed = Some((until, reason.clone()));
                self.audit("arm", json!({ "reason": reason, "minutes": minutes }));
                tell(sink, SafetyEvent::Armed { minutes }).await;
            }
            SafetyCommand::Disarm { expired } => {
                if self.armed.is_some() {
                    let why = if expired { "expired" } else { "user" };
                    self.audit("disarm", json!({ "why": why }));
                    self.armed = None;
                }
                tell(sink, SafetyEvent::Disarmed).await;
            }
            SafetyCommand::Declined { preview } => {
                let items: Vec<Value> = preview.items.iter().map(item_json).collect();
                self.audit(
                    "declined",
                    json!({ "items": items, "tier": gate_name(&preview.gate) }),
                );
            }
            SafetyCommand::Prepare { req, specs, batch } => {
                if let Some(why) = self.refusal() {
                    return self.refuse(sink, Some(req), "prepare", why).await;
                }
                let event = match self.prepare(client, rate, &specs, batch).await {
                    Ok(preview) => SafetyEvent::Preview {
                        req,
                        preview: Box::new(preview),
                    },
                    Err(reason) => SafetyEvent::Refused {
                        req: Some(req),
                        reason,
                    },
                };
                tell(sink, event).await;
            }
            SafetyCommand::Execute { req, preview } => {
                let refusal = self.refusal().or_else(|| match &preview.gate {
                    Gate::Blocked(why) => Some(why.clone()),
                    Gate::Allowed(_) => None,
                });
                let refusal = refusal.or_else(|| {
                    let six = preview.items.iter().any(|i| i.priority == Some(6));
                    (six && !self.options.policy.dangerous).then(|| PRIORITY_6.to_string())
                });
                if let Some(why) = refusal {
                    return self.refuse(sink, Some(req), "write", why).await;
                }
                let outcomes = self.execute(client, sink, rate, &preview).await;
                // The ledger first, so the UI never shows a result without it.
                tell(sink, SafetyEvent::Ledger(self.entries())).await;
                tell(sink, SafetyEvent::Done { req, outcomes }).await;
            }
            SafetyCommand::Revert { req } => {
                let refusal = if self.options.passive {
                    Some(PASSIVE.to_string())
                } else if self.store.is_none() {
                    Some(self.missing.clone())
                } else {
                    None
                };
                if let Some(why) = refusal {
                    return self.refuse(sink, Some(req), "revert", why).await;
                }
                let failed = self.revert(client, sink, rate).await;
                tell(sink, SafetyEvent::Ledger(self.entries())).await;
                tell(sink, SafetyEvent::Reverted { req, failed }).await;
            }
            SafetyCommand::Keep => {
                let labels: Vec<String> = self.entries().iter().map(Entry::label).collect();
                if let Some((ledger, _)) = &mut self.store {
                    ledger.keep_all();
                }
                self.save();
                self.audit("kept", json!({ "entries": labels }));
                tell(sink, SafetyEvent::Ledger(self.entries())).await;
            }
        }
    }

    /// Read what `specs` need and plan them.
    async fn prepare<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        rate: &RateCap,
        specs: &[WriteSpec],
        batch: bool,
    ) -> Result<Preview, String> {
        let device = specs.first().ok_or("nothing to write")?.device;
        if specs.iter().any(|spec| spec.device != device) {
            return Err("a batch goes to one device".into());
        }
        let cancel = AtomicBool::new(false);
        let mut reader = Reader {
            client,
            device,
            rate,
            cancel: &cancel,
            rpm: true,
        };
        let mut rows = Vec::with_capacity(specs.len());
        for spec in specs {
            let mut wanted = PRE_READ.to_vec();
            if !wanted.contains(&spec.property) && spec.index.is_none() {
                wanted.push(spec.property);
            }
            match reader.properties(spec.object, Wanted::List(&wanted)).await {
                Ok((read, _)) => rows.push(read),
                Err(Halt::Device(error)) => {
                    return Err(format!("reading {}: {error}", spec.object))
                }
                Err(Halt::Cancelled) => return Err("cancelled".into()),
            }
        }
        let ledger = self.store.as_ref().map(|(ledger, _)| ledger);
        let ours = |spec: &WriteSpec, priority| ledger.is_some_and(|l| l.holds(spec, priority));
        plan::preview(specs, &rows, ours, self.options.policy.dangerous, batch)
    }

    /// Send a confirmed preview, ledger first, and record each outcome.
    async fn execute<T: TransportPort + 'static>(
        &mut self,
        client: &BACnetClient<T>,
        sink: &EventSink,
        rate: &RateCap,
        preview: &Preview,
    ) -> Vec<(String, Outcome)> {
        let labels = preview.items.iter().map(Planned::label);
        let not_sent = |runner: &mut Self, why: &str| {
            let outcome = Outcome::NotSent(why.into());
            for item in &preview.items {
                runner.audit_write(preview, item, &outcome);
            }
            labels
                .clone()
                .map(|label| (label, outcome.clone()))
                .collect()
        };
        if sink.is_cancelled() {
            // The session ended before this was sent: record it, send nothing.
            return not_sent(self, "the session ended first");
        }
        let Some((ledger, _)) = &mut self.store else {
            return Vec::new();
        };
        let backup = ledger.entries().to_vec();
        let at = utc_now();
        let ids: Vec<u64> = preview
            .items
            .iter()
            .map(|item| ledger.begin(item, run_id(), &at))
            .collect();
        if !self.save() {
            // Nothing goes out that the ledger couldn't record first.
            if let Some((ledger, _)) = &mut self.store {
                for id in &ids {
                    ledger.finish(*id, Some(false), &backup);
                }
            }
            return not_sent(self, "the ledger could not be saved");
        }
        rate.acquire().await;
        let outcomes = send(client, preview).await;
        for ((item, id), outcome) in preview.items.iter().zip(&ids).zip(&outcomes) {
            if let Some((ledger, _)) = &mut self.store {
                ledger.finish(*id, outcome.applied(), &backup);
            }
            self.audit_write(preview, item, outcome);
        }
        self.save();
        labels.zip(outcomes).collect()
    }

    fn audit_write(&mut self, preview: &Preview, item: &Planned, outcome: &Outcome) {
        let service = if preview.batch {
            "WritePropertyMultiple"
        } else {
            "WriteProperty"
        };
        let event = if item.spec.value.is_none() {
            "relinquish"
        } else {
            "write"
        };
        let mut fields = item_json(item);
        if let Value::Object(map) = &mut fields {
            map.insert("service".into(), json!(service));
            map.insert("tier".into(), json!(gate_name(&preview.gate)));
            map.insert("outcome".into(), outcome.json());
        }
        self.audit(event, fields);
    }

    /// Undo every entry, newest first; returns how many are left.
    async fn revert<T: TransportPort + 'static>(
        &mut self,
        client: &BACnetClient<T>,
        sink: &EventSink,
        rate: &RateCap,
    ) -> usize {
        let mut entries = self.entries();
        entries.reverse();
        for entry in entries {
            let outcome = match (entry.undo(), entry.object()) {
                _ if sink.is_cancelled() => Outcome::NotSent("the session ended first".into()),
                (Some(undo), Some(object)) => {
                    rate.acquire().await;
                    let result = client
                        .write_property_to_device(
                            entry.device,
                            object,
                            entry.property(),
                            entry.index,
                            undo.bytes.clone(),
                            entry.priority,
                        )
                        .await;
                    Outcome::of(&result)
                }
                _ => Outcome::NotSent("the old value is unknown".into()),
            };
            if outcome.applied() == Some(true) {
                if let Some((ledger, _)) = &mut self.store {
                    ledger.remove(entry.id);
                }
                self.save();
            }
            self.audit(
                "revert",
                json!({
                    "target": entry.label(),
                    "service": "WriteProperty",
                    "priority": entry.priority,
                    "params": { "value": entry.undo().map(|u| u.text) },
                    "pre_image": { "written": entry.written.text },
                    "outcome": outcome.json(),
                }),
            );
        }
        self.entries().len()
    }
}

/// Send the preview as one WriteProperty or one WritePropertyMultiple, and
/// give each write its outcome. A WritePropertyMultiple error names the
/// first write that failed: the ones before it took effect and the rest
/// were not tried (Clause 15.10.2).
async fn send<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    preview: &Preview,
) -> Vec<Outcome> {
    let items = &preview.items;
    if !preview.batch {
        let Some(item) = items.first() else {
            return Vec::new();
        };
        let spec = &item.spec;
        let result = client
            .write_property_to_device(
                spec.device,
                spec.object,
                spec.property,
                spec.index,
                item.value.bytes.clone(),
                item.priority,
            )
            .await;
        return vec![Outcome::of(&result)];
    }
    let mut specs: Vec<WriteAccessSpecification> = Vec::new();
    for item in items {
        let value = BACnetPropertyValue {
            property_identifier: item.spec.property,
            property_array_index: item.spec.index,
            value: item.value.bytes.clone(),
            priority: item.priority,
        };
        match specs.last_mut() {
            Some(last) if last.object_identifier == item.spec.object => {
                last.list_of_properties.push(value);
            }
            _ => specs.push(WriteAccessSpecification {
                object_identifier: item.spec.object,
                list_of_properties: vec![value],
            }),
        }
    }
    let result = client
        .write_property_multiple_to_device(preview.device, specs)
        .await;
    let failed_at = match &result {
        Err(Error::Structured { detail, .. }) => match detail.as_ref() {
            ErrorDetail::FirstFailedWriteAttempt(at) => items.iter().position(|item| {
                item.spec.object == at.object_identifier
                    && item.spec.property.to_raw() == at.property_identifier
                    && item.spec.index == at.property_array_index
            }),
            _ => None,
        },
        _ => None,
    };
    let outcome = match (&result, failed_at) {
        // An Error PDU without the failed write leaves which ones took
        // effect unknown.
        (Err(Error::Protocol { .. } | Error::Structured { .. }), None) => Outcome::Failed(
            result
                .as_ref()
                .err()
                .map(Error::to_string)
                .unwrap_or_default(),
        ),
        _ => Outcome::of(&result),
    };
    (0..items.len())
        .map(|i| match failed_at {
            Some(at) if i < at => Outcome::Ack,
            Some(at) if i > at => Outcome::NotSent("an earlier write in the batch failed".into()),
            _ => outcome.clone(),
        })
        .collect()
}

fn gate_name(gate: &Gate) -> String {
    match gate {
        Gate::Allowed(tier) => format!("{tier:?}"),
        Gate::Blocked(_) => "blocked".into(),
    }
}

/// A planned write's audit fields.
fn item_json(item: &Planned) -> Value {
    let spec = &item.spec;
    json!({
        "target": {
            "device": spec.device,
            "object": crate::tui::app::objects::object_label(spec.object),
            "property": crate::tui::app::objects::kebab(spec.property),
            "index": spec.index,
        },
        "params": { "value": item.value.text, "encoded": hex(&item.value.bytes) },
        "priority": item.priority,
        "pre_image": {
            "value": item.before.as_ref().map(|b| b.text.clone()),
            "encoded": item.before.as_ref().map(|b| hex(&b.bytes)),
            "out_of_service": item.out_of_service,
        },
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
