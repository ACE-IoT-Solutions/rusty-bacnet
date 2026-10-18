//! `--passive`: a session that transmits nothing.
//!
//! In production a passive session builds no client at all, so nothing can
//! reach the wire, not even an automatic answer to an incoming request. As
//! a second line, the session loop of a built client also refuses anything
//! that transmits when its options say passive.

use tokio::sync::mpsc;

use super::write::{ready_offline, SafetyOptions, PASSIVE};
use super::EventSink;
use crate::tui::message::{Command, OpOutcome, WorkerEvent};
use crate::tui::point::BrowseResult;
use crate::tui::safety::{SafetyCommand, SafetyEvent};
use crate::tui::watched::{Source, WatchCommand, WatchEvent};

/// Serve a passive session: list the ledger, then refuse every request
/// until the session ends.
pub(crate) async fn serve(
    sink: &EventSink,
    mut commands: mpsc::Receiver<Command>,
    options: &SafetyOptions,
) {
    tracing::info!("passive: no network client; nothing is transmitted");
    if !sink.deliver(WorkerEvent::Passive).await {
        return;
    }
    ready_offline(sink, options).await;
    loop {
        tokio::select! {
            biased;
            () = sink.cancelled() => break,
            command = commands.recv() => match command {
                None | Some(Command::Disconnect) => break,
                Some(command) => refuse(sink, command).await,
            },
        }
    }
}

/// True for a command that would put something on the wire.
pub(crate) fn transmits(command: &Command) -> bool {
    matches!(
        command,
        Command::WhoIs { .. } | Command::Browse { .. } | Command::Watch(_) | Command::Safety(_)
    )
}

/// Answer a command a passive session won't carry out, so nothing waits
/// for it.
pub(crate) async fn refuse(sink: &EventSink, command: Command) {
    let reason = PASSIVE.to_string();
    let event = match command {
        Command::WhoIs { op, .. } => WorkerEvent::OpFinished {
            op,
            outcome: OpOutcome::Failed(reason),
        },
        Command::Browse { req, .. } => WorkerEvent::Browse {
            req,
            result: BrowseResult::Failed(reason),
        },
        Command::Watch(WatchCommand::Add { row, .. }) => WorkerEvent::Watch(WatchEvent::Source {
            row,
            source: Source::Failed(reason),
        }),
        Command::Safety(
            SafetyCommand::Prepare { req, .. }
            | SafetyCommand::Execute { req, .. }
            | SafetyCommand::Revert { req },
        ) => WorkerEvent::Safety(SafetyEvent::Refused {
            req: Some(req),
            reason,
        }),
        Command::Safety(SafetyCommand::Arm { .. }) => {
            WorkerEvent::Safety(SafetyEvent::Refused { req: None, reason })
        }
        _ => return,
    };
    sink.deliver(event).await;
}
