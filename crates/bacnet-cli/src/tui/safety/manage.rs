//! Device management from the TUI (#1799): DeviceCommunicationControl
//! (`:dcc`) and ReinitializeDevice (`:reinit`), each to one device.
//!
//! Both are the one-shot guard's device-management class: they need an
//! armed write session on a profile with `dangerous = true`, and always
//! take the T3 confirmation (the device instance typed out). The password,
//! when one is given, comes from a masked prompt and lives only in a
//! [`Secret`]: the audit log, the screen and the logs never see its text.
//! Neither service is kept in the ledger, since neither can be undone by
//! writing an old value back.

use bacnet_types::enums::{EnableDisable, ReinitializedState};
use serde_json::{Map, Value, json};

use super::audit::Secret;
use super::{Gate, Tier};
use crate::commands::device::{DCC_ACTIONS, REINIT_STATES};

/// The longest password either service carries, in characters
/// (Clauses 16.1.1.1.3 and 16.4.1.1.2).
pub(crate) const MAX_PASSWORD: usize = 20;

/// Why a profile without the dangerous opt-in may not send either service.
pub(crate) const NEEDS_DANGEROUS: &str = "device management (dcc, reinit) needs a profile with \
     dangerous = true in its [safety] table";

/// What to ask the device to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// DeviceCommunicationControl.
    Dcc {
        /// Enable, disable, or disable initiation.
        state: EnableDisable,
        /// How long, in minutes; `None` is until told otherwise.
        minutes: Option<u16>,
    },
    /// ReinitializeDevice.
    Reinit {
        /// Which restart or backup step.
        state: ReinitializedState,
    },
}

/// `value`'s name in `table`, or its own display (a number for one
/// without a name).
fn name_in<T: Copy + PartialEq + std::fmt::Display>(
    table: &[(&'static str, T)],
    value: T,
) -> String {
    table
        .iter()
        .find(|(_, v)| *v == value)
        .map_or_else(|| value.to_string(), |(name, _)| (*name).to_string())
}

impl Action {
    /// The service's name.
    pub(crate) fn service(self) -> &'static str {
        match self {
            Self::Dcc { .. } => "DeviceCommunicationControl",
            Self::Reinit { .. } => "ReinitializeDevice",
        }
    }

    /// The command bar word, which is also the audit event.
    pub(crate) fn command(self) -> &'static str {
        match self {
            Self::Dcc { .. } => "dcc",
            Self::Reinit { .. } => "reinit",
        }
    }

    /// The state as the command bar names it.
    pub(crate) fn state(self) -> String {
        match self {
            Self::Dcc { state, .. } => name_in(&DCC_ACTIONS, state),
            Self::Reinit { state } => name_in(&REINIT_STATES, state),
        }
    }

    /// `disable for 30 min`, `coldstart`.
    pub(crate) fn describe(self) -> String {
        match self {
            Self::Dcc {
                minutes: Some(minutes),
                ..
            } => format!("{} for {minutes} min", self.state()),
            Self::Dcc { minutes: None, .. } => format!("{} until told otherwise", self.state()),
            Self::Reinit { .. } => self.state(),
        }
    }
}

/// One device management request, ready to confirm and send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    /// The device instance.
    pub(crate) device: u32,
    /// What to ask.
    pub(crate) action: Action,
    /// The password; `None` sends none.
    pub(crate) password: Option<Secret>,
}

impl Request {
    /// `DeviceCommunicationControl to device 100: disable for 30 min`.
    pub(crate) fn label(&self) -> String {
        format!(
            "{} to device {}: {}",
            self.action.service(),
            self.device,
            self.action.describe()
        )
    }

    /// Always T3.
    pub(crate) fn gate(&self) -> Gate {
        Gate::Allowed(Tier::T3)
    }

    /// The audit fields: service, target, parameters and tier. A password
    /// shows only as present, its value written as `[redacted]`.
    pub(crate) fn audit_json(&self) -> Value {
        let mut params = Map::new();
        params.insert("state".into(), json!(self.action.state()));
        if let Action::Dcc { minutes, .. } = self.action {
            params.insert("minutes".into(), json!(minutes));
        }
        if let Some(password) = &self.password {
            params.insert("password".into(), json!(password));
        }
        json!({
            "service": self.action.service(),
            "target": { "device": self.device },
            "params": params,
            "tier": "T3",
        })
    }

    /// The request without its password, for a decline: what was declined
    /// is recorded, and the password goes no further.
    pub(crate) fn forget_password(&mut self) {
        self.password = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_name_their_service_state_and_device() {
        let dcc = Request {
            device: 100,
            action: Action::Dcc {
                state: EnableDisable::DISABLE_INITIATION,
                minutes: Some(30),
            },
            password: Some(Secret::new("hunter2-unit")),
        };
        assert_eq!(
            dcc.label(),
            "DeviceCommunicationControl to device 100: disable-initiation for 30 min"
        );
        let reinit = Request {
            device: 7,
            action: Action::Reinit {
                state: ReinitializedState::ACTIVATE_CHANGES,
            },
            password: None,
        };
        assert_eq!(
            reinit.label(),
            "ReinitializeDevice to device 7: activate-changes"
        );
        // A state without a name shows its number.
        let odd = Action::Reinit {
            state: ReinitializedState::from_raw(99),
        };
        assert_eq!(odd.state(), "99");
    }

    /// Neither Debug output nor the audit fields hold the password text.
    #[test]
    fn the_password_shows_only_as_present() {
        let mut dcc = Request {
            device: 100,
            action: Action::Dcc {
                state: EnableDisable::DISABLE,
                minutes: None,
            },
            password: Some(Secret::new("hunter2-unit")),
        };
        let debug = format!("{dcc:?}");
        assert!(!debug.contains("hunter2"), "{debug}");
        let fields = dcc.audit_json();
        assert!(!fields.to_string().contains("hunter2"), "{fields}");
        assert_eq!(fields["params"]["password"], "[redacted]");
        assert_eq!(fields["params"]["minutes"], Value::Null);
        assert_eq!(fields["params"]["state"], "disable");
        dcc.forget_password();
        assert!(dcc.audit_json()["params"].get("password").is_none());
    }
}
