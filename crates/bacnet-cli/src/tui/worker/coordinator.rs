//! One retained owner for connection attempts, commands and complete retirement.

use super::session::{ConnectPlan, TransportKind};
use super::{session, EventSink, COMMAND_CHANNEL_CAPACITY};
use crate::tui::message::{Command, OpId, WorkerEvent};
use crate::tui::point::{BrowseResult, ReqId};
use crate::tui::profile::service::{Reply, Request, Service};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

struct Active {
    cancel: watch::Sender<bool>,
    commands: mpsc::Sender<Command>,
    task: JoinHandle<()>,
}

/// The coordinator lives until the UI drops its command channel. It never
/// detaches an attempt or starts a replacement before the old one is joined.
/// `max_rate` caps the device browser's requests a second (0: no cap).
pub(crate) fn spawn(
    plan: Option<ConnectPlan>,
    profiles: Option<Service>,
    sink: EventSink,
    commands: mpsc::Receiver<Command>,
    max_rate: u32,
) -> JoinHandle<()> {
    tokio::spawn(Box::pin(run(plan, profiles, sink, commands, max_rate)))
}

async fn start(
    plan: &ConnectPlan,
    generation: u64,
    sink: &EventSink,
    max_rate: u32,
) -> Option<Active> {
    let (cancel, cancelled) = watch::channel(false);
    let (commands, command_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
    if !sink
        .deliver(WorkerEvent::SessionStarted { generation })
        .await
    {
        return None;
    }
    let scoped = sink.for_session(generation, cancelled);
    let task = session::spawn(plan.clone(), scoped, command_rx, max_rate);
    Some(Active {
        cancel,
        commands,
        task,
    })
}

async fn retire(active: &mut Option<Active>) -> Result<(), String> {
    if let Some(Active {
        cancel,
        commands,
        task,
    }) = active.take()
    {
        cancel.send_replace(true);
        drop(commands);
        task.await
            .map_err(|error| format!("network session panicked: {error}"))?;
    }
    Ok(())
}

async fn completed(active: &mut Option<Active>) -> Result<(), tokio::task::JoinError> {
    match active {
        Some(active) => (&mut active.task).await,
        None => std::future::pending().await,
    }
}

async fn run(
    mut plan: Option<ConnectPlan>,
    profiles: Option<Service>,
    sink: EventSink,
    mut commands: mpsc::Receiver<Command>,
    max_rate: u32,
) {
    let mut generation = 0;
    let mut active = None;
    if let Some(plan) = &plan {
        if !plan.await_interface {
            generation += 1;
            active = start(plan, generation, &sink, max_rate).await;
        }
    }
    loop {
        tokio::select! {
            done = completed(&mut active), if active.is_some() => {
                active = None;
                if let Err(error) = done {
                    sink.deliver(WorkerEvent::Fatal { error: format!("network worker failed: {error}") }).await;
                    break;
                }
                // A failed attempt already reported its error. Keep the
                // coordinator alive so the user can edit and retry.
            }
            command = commands.recv() => {
                let Some(command) = command else { break; };
                if active.as_ref().is_some_and(|active| active.task.is_finished()) {
                    if let Err(error) = retire(&mut active).await {
                        sink.deliver(WorkerEvent::Fatal { error }).await;
                        break;
                    }
                }
                match command {
                    Command::Disconnect => {
                        if let Err(error) = retire(&mut active).await {
                            sink.deliver(WorkerEvent::Fatal { error }).await;
                            break;
                        }
                        sink.deliver(WorkerEvent::Disconnected { generation }).await;
                    }
                    Command::WhoIs { op, .. } | Command::Cancel { op } => {
                        forward_operation(active.as_ref(), command, op, generation, &sink).await;
                    }
                    Command::Browse { req, .. } => {
                        forward_browse(active.as_ref(), command, req, generation, &sink).await;
                    }
                    Command::StopBrowse { .. } => {
                        // Best effort: a stale browse result is dropped by the UI anyway.
                        if let Some(active) = &active {
                            if let Err(error) = active.commands.try_send(command) {
                                tracing::debug!("browse stop not admitted: {error}");
                            }
                        }
                    }
                    Command::Profile(request) => {
                        let Some(profiles) = &profiles else {
                            sink.deliver(WorkerEvent::Profile(Err("profiles are disabled in this flags-only session".into()))).await;
                            continue;
                        };
                        // Saving/loading another configuration while active
                        // would make the displayed selection differ from the
                        // live connection. Retire explicitly first.
                        if active.is_some() {
                            sink.deliver(WorkerEvent::Profile(Err("disconnect before changing or connecting a profile".into()))).await;
                            continue;
                        }
                        let result = match request {
                            Request::List => profiles.catalog(),
                            Request::Load(name) => profiles.load(&name),
                            Request::Save { name, profile, create, previous } => profiles.save(&name, profile, create, previous),
                            Request::Connect { name, mut profile, attest } => {
                                match profiles.prepare(&name, &profile, attest).and_then(|resolved| Ok((resolved.transport_args()?, resolved.authorized_at_unix))) {
                                    Ok((args, authorized_at_unix)) => {
                                        // Publish the persisted snapshot used by this attempt,
                                        // without reloading a possibly newer external edit.
                                        profile.authorized_at_unix = authorized_at_unix;
                                        sink.deliver(WorkerEvent::Profile(Ok(Reply::Loaded { name, profile }))).await;
                                        let next = ConnectPlan { kind: TransportKind::Bip, args, await_interface: false };
                                        generation += 1;
                                        active = start(&next, generation, &sink, max_rate).await;
                                        plan = Some(next);
                                        continue;
                                    }
                                    Err(error) => Err(error),
                                }
                            }
                        };
                        sink.deliver(WorkerEvent::Profile(result)).await;
                    }
                    Command::Connect { interface, broadcast } => {
                        if active.is_some() { reject(&sink, "disconnect before reconnecting").await; continue; }
                        if let Some(plan) = &mut plan {
                            plan.args.interface = interface; plan.args.broadcast = broadcast;
                            plan.await_interface = false;
                            generation += 1;
                            active = start(plan, generation, &sink, max_rate).await;
                        } else { reject(&sink, "select a profile before connecting").await; }
                    }
                    Command::Reconnect => {
                        if profiles.is_some() { reject(&sink, "review the profile before reconnecting").await; continue; }
                        if active.is_some() { reject(&sink, "disconnect before reconnecting").await; continue; }
                        if let Some(plan) = &plan {
                            if plan.await_interface { reject(&sink, "choose an interface first").await; continue; }
                            generation += 1;
                            active = start(plan, generation, &sink, max_rate).await;
                        }
                    }
                }
            }
        }
    }
    if let Err(error) = retire(&mut active).await {
        tracing::error!("{error}");
    }
}

async fn reject(sink: &EventSink, error: &str) {
    sink.deliver(WorkerEvent::Rejected {
        error: error.into(),
    })
    .await;
}

/// Forward across the second bounded queue, retaining operation/session identity.
async fn forward_operation(
    active: Option<&Active>,
    command: Command,
    op: OpId,
    generation: u64,
    sink: &EventSink,
) {
    let cancel = matches!(command, Command::Cancel { .. });
    let error = match active {
        Some(active) => active
            .commands
            .try_send(command)
            .err()
            .map(|e| format!("session command not admitted: {e}")),
        None => Some("no active connection".into()),
    };
    if let Some(error) = error {
        sink.deliver(WorkerEvent::Session {
            generation,
            event: Box::new(WorkerEvent::OpRejected { op, cancel, error }),
        })
        .await;
    }
}

/// Forward a browse read; one that can't be admitted fails at once, under
/// its request id, so the browser doesn't wait for it.
async fn forward_browse(
    active: Option<&Active>,
    command: Command,
    req: ReqId,
    generation: u64,
    sink: &EventSink,
) {
    let error = match active {
        Some(active) => active
            .commands
            .try_send(command)
            .err()
            .map(|e| format!("session command not admitted: {e}")),
        None => Some("no active connection".into()),
    };
    if let Some(error) = error {
        sink.deliver(WorkerEvent::Session {
            generation,
            event: Box::new(WorkerEvent::Browse {
                req,
                result: BrowseResult::Failed(error),
            }),
        })
        .await;
    }
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
