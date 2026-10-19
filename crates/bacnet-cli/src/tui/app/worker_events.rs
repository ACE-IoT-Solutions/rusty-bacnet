//! What the worker reports, and the commands it refused: the transitions
//! [`App`] makes for each [`WorkerEvent`], kept apart from `update`'s key
//! handling.

use super::*;

impl App {
    /// Channel admission is part of the UI transition: never silently report
    /// a disconnected or connecting state for a command that was dropped.
    pub(crate) fn command_rejected(&mut self, command: Command, prior_link: Link, error: String) {
        self.link = prior_link;
        if let Command::Profile(_) = command {
            if let Some(connection) = &mut self.connection {
                connection.reply(Err(error.clone()));
            }
        }
        if let Command::WhoIs { op, .. } | Command::Cancel { op } = command {
            self.operation_rejected(op, matches!(command, Command::Cancel { .. }), error);
        } else {
            self.flash(error, FLASH_FOR);
        }
        self.dirty = true;
    }
    /// Only admission failure may correct an optimistic cancellation. Ordinary
    /// late progress/completion continues to require a running operation.
    fn operation_rejected(&mut self, id: OpId, cancel: bool, error: String) {
        let Some(op) = self.op.as_mut().filter(|op| {
            op.id == id
                && if cancel {
                    matches!(op.state, OpState::Cancelled | OpState::CancelFailed(_))
                } else {
                    op.running()
                }
        }) else {
            return;
        };
        if cancel {
            op.state = OpState::CancelFailed(error);
            self.ctrl_c_at = None;
            self.flash("Cancel refused. Ctrl-C to retry; x disconnects.", FLASH_FOR);
        } else {
            op.state = OpState::Failed(error.clone());
            self.flash(error, FLASH_FOR);
        }
    }

    pub(super) fn worker_event(&mut self, event: WorkerEvent, out: &mut Vec<Command>) {
        match event {
            WorkerEvent::Fatal { error } => self.fail(error, 1),
            WorkerEvent::Session { generation, event } => {
                if generation == self.generation && self.link != Link::Disconnecting {
                    self.worker_event(*event, out);
                }
            }
            WorkerEvent::SessionStarted { generation } => {
                self.generation = generation;
                self.reset_session();
                if self.link != Link::Disconnecting {
                    self.link = Link::Connecting;
                }
                self.overlay = Overlay::None;
            }
            WorkerEvent::Disconnected { generation } => {
                if generation == self.generation {
                    self.reset_session();
                    self.link = Link::Down;
                    self.flash("Disconnected; local resources released.", FLASH_FOR);
                }
            }
            WorkerEvent::Profile(result) => {
                if let Err(error) = &result {
                    if self.link == Link::Connecting {
                        self.link = Link::Failed(error.clone());
                    }
                }
                if let Some(connection) = &mut self.connection {
                    connection.reply(result);
                }
            }
            WorkerEvent::Rejected { error } => {
                if self.link == Link::Connecting {
                    self.link = Link::Failed(error.clone());
                }
                if let Some(op) = self.op.as_mut().filter(|op| op.running()) {
                    op.state = OpState::Failed(error.clone());
                }
                self.flash(error, FLASH_FOR);
            }
            WorkerEvent::Connected { local } => self.link = Link::Up { local },
            WorkerEvent::ConnectFailed { error } => {
                self.link = Link::Failed(error.clone());
                self.flash(error, FLASH_FOR);
            }
            WorkerEvent::Discovered(row) | WorkerEvent::Updated(row) => {
                if let Some(op) = self.op.as_mut().filter(|op| op.running()) {
                    op.replies += usize::from(op.spec.wants(row.instance));
                }
                self.devices.upsert(row);
            }
            WorkerEvent::Lost(row) => self.devices.remove(row.instance),
            WorkerEvent::Collision { retained, incoming } => {
                self.duplicates.record(&retained, &incoming);
            }
            WorkerEvent::Snapshot(rows) => self.devices.replace_all(rows),
            WorkerEvent::WhoIsSent { op } => {
                let now = self.now;
                if let Some(op) = self.running_op_mut(op) {
                    op.state = OpState::Listening {
                        until: now + op.spec.listen,
                    };
                }
            }
            WorkerEvent::OpRejected { op, cancel, error } => {
                self.operation_rejected(op, cancel, error)
            }
            WorkerEvent::Watch(event) => {
                if let Some(text) = self.watch.apply(event) {
                    self.flash(text, FLASH_FOR);
                }
            }
            WorkerEvent::Browse { req, result } => {
                if let Some(browser) = self.browser.as_deref_mut() {
                    browser.apply(req, result);
                }
            }
            WorkerEvent::OpFinished { op, outcome } => {
                if let Some(op) = self.running_op_mut(op) {
                    op.state = match outcome {
                        OpOutcome::Completed => OpState::Done,
                        OpOutcome::Cancelled => OpState::Cancelled,
                        OpOutcome::Failed(error) => OpState::Failed(error),
                    };
                }
            }
            WorkerEvent::Safety(event) => self.safety_event(event, out),
            WorkerEvent::Passive => self.link = Link::Passive,
            WorkerEvent::ForeignDevice(foreign) => self.network.foreign_device(foreign),
            WorkerEvent::Network(event) => self.network_event(event),
            WorkerEvent::Discovery(event) => self.discovery_event(event),
            WorkerEvent::Sc(report) => self.sc_report(report),
        }
    }
}
