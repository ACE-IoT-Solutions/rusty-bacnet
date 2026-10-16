//! The profile picker, editor and resolved preview. No filesystem or network I/O.

use crate::tui::message::Command;
use crate::tui::profile::service::{Reply, Request};
use crate::tui::profile::{Overrides, Profile, Writes};

/// Editing keys have no transport effects until a request is emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Key {
    Next,
    Prev,
    Char(char),
    Backspace,
    Discard,
    Submit,
    New,
    Edit,
    Reload,
    List,
    Connect,
    Attest,
}

/// One of the connection dialog's three screens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Screen {
    List,
    Edit,
    Preview,
}

/// Text fields have a fixed order shared by the editor and its renderer.
pub(crate) const LABELS: [&str; 10] = [
    "Name",
    "Interface",
    "UDP port",
    "Broadcast",
    "Timeout ms",
    "Pacing ms",
    "BBMD",
    "TTL seconds",
    "Renewal sec",
    "Writes",
];

/// State retained while a connection is running or the dialog is closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Connection {
    pub(crate) names: Vec<String>,
    pub(crate) selected: usize,
    pub(crate) root: String,
    pub(crate) screen: Screen,
    pub(crate) name: String,
    pub(crate) saved: Option<Profile>,
    pub(crate) fields: [String; 10],
    pub(crate) focus: usize,
    pub(crate) creating: bool,
    pub(crate) error: Option<String>,
    pub(crate) overrides: Overrides,
    /// A queued disk operation disables further mutations until its reply.
    pub(crate) pending: bool,
}

impl Connection {
    pub(crate) fn new(overrides: Overrides) -> Self {
        Self {
            names: Vec::new(),
            selected: 0,
            root: String::new(),
            screen: Screen::List,
            name: String::new(),
            saved: None,
            fields: Default::default(),
            focus: 0,
            creating: false,
            error: None,
            overrides,
            pending: false,
        }
    }
    pub(crate) fn reply(&mut self, reply: Result<Reply, String>) {
        self.pending = false;
        self.error = None;
        match reply {
            Ok(Reply::Catalog { names, root }) => {
                self.names = names;
                self.root = root;
                self.selected = self.selected.min(self.names.len().saturating_sub(1));
                self.screen = Screen::List;
            }
            Ok(Reply::Loaded { name, profile }) => {
                if !self.names.contains(&name) {
                    self.names.push(name.clone());
                    self.names.sort();
                }
                self.name = name;
                self.saved = Some(profile);
                self.creating = false;
                self.screen = Screen::Preview;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(crate) fn new_profile(&mut self) {
        self.name.clear();
        self.saved = None;
        self.creating = true;
        self.edit(Profile::default());
    }
    fn edit(&mut self, p: Profile) {
        let b = p.bip;
        self.fields = [
            self.name.clone(),
            b.interface.to_string(),
            b.port.to_string(),
            b.broadcast.to_string(),
            b.timeout_ms.to_string(),
            b.min_interval_ms.to_string(),
            b.bbmd.map(|v| v.to_string()).unwrap_or_default(),
            b.ttl.to_string(),
            b.renewal_secs.map(|v| v.to_string()).unwrap_or_default(),
            match p.safety.writes {
                Writes::Never => "never",
                Writes::Session => "session",
            }
            .into(),
        ];
        self.focus = usize::from(!self.creating);
        self.screen = Screen::Edit;
        self.error = None;
    }
    fn edited(&self) -> Result<Profile, String> {
        super::super::profile::store::validate_name(&self.fields[0])?;
        let mut profile = self.saved.clone().unwrap_or_default();
        let f = &self.fields;
        profile.bip.interface = f[1]
            .parse()
            .map_err(|_| "Interface must be an IPv4 address")?;
        profile.bip.port = f[2].parse().map_err(|_| "UDP port must be 0-65535")?;
        profile.bip.broadcast = f[3]
            .parse()
            .map_err(|_| "Broadcast must be an IPv4 address")?;
        profile.bip.timeout_ms = f[4]
            .parse()
            .map_err(|_| "Timeout must be an unsigned number")?;
        profile.bip.min_interval_ms = f[5]
            .parse()
            .map_err(|_| "Pacing must be an unsigned number")?;
        profile.bip.bbmd = if f[6].is_empty() {
            None
        } else {
            Some(crate::foreign::parse_bbmd_arg(&f[6])?)
        };
        profile.bip.ttl = f[7].parse().map_err(|_| "TTL must be 0-65535")?;
        profile.bip.renewal_secs = if f[8].is_empty() {
            None
        } else {
            Some(
                f[8].parse()
                    .map_err(|_| "Renewal must be an unsigned number")?,
            )
        };
        if profile.bip.bbmd.is_none() {
            profile.bip.renewal_secs = None;
        }
        profile.safety.writes = match f[9].as_str() {
            "never" => Writes::Never,
            "session" => Writes::Session,
            _ => return Err("Writes must be never or session; this TUI remains read-only".into()),
        };
        if self
            .saved
            .as_ref()
            .is_some_and(|saved| saved.bip != profile.bip)
        {
            profile.authorized_at_unix = None;
        }
        profile.validate()?;
        self.overrides.resolve(&profile)?;
        Ok(profile)
    }
    pub(crate) fn resolved(&self) -> Result<Profile, String> {
        self.overrides
            .resolve(self.saved.as_ref().ok_or("No profile selected")?)
    }
    /// Return at most one observable, acknowledged coordinator request.
    pub(crate) fn key(&mut self, key: Key) -> Option<Command> {
        if self.pending {
            return None;
        }
        self.error = None;
        let request = match (&self.screen, key) {
            (Screen::Edit, Key::Next) => {
                self.focus = (self.focus + 1) % 10;
                if !self.creating && self.focus == 0 {
                    self.focus = 1;
                }
                None
            }
            (Screen::Edit, Key::Prev) => {
                self.focus = (self.focus + 9) % 10;
                if !self.creating && self.focus == 0 {
                    self.focus = 9;
                }
                None
            }
            (Screen::Edit, Key::Char(ch)) => {
                if self.fields[self.focus].len() < 128 && ch.is_ascii() && !ch.is_control() {
                    self.fields[self.focus].push(ch);
                }
                None
            }
            (Screen::Edit, Key::Backspace) => {
                self.fields[self.focus].pop();
                None
            }
            (Screen::Edit, Key::Discard) => {
                self.fields = Default::default();
                self.creating = false;
                self.screen = if self.saved.is_some() {
                    Screen::Preview
                } else {
                    Screen::List
                };
                None
            }
            (Screen::Edit, Key::Submit) => match self.edited() {
                Ok(profile) => Some(Request::Save {
                    name: self.fields[0].clone(),
                    profile,
                    create: self.creating,
                    previous: self.saved.clone(),
                }),
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            },
            (_, Key::New) => {
                self.new_profile();
                None
            }
            (Screen::Preview, Key::Edit) => {
                if let Some(profile) = &self.saved {
                    self.edit(profile.clone());
                }
                None
            }
            (_, Key::List) => Some(Request::List),
            (Screen::List, Key::Reload) => Some(Request::List),
            (Screen::List, Key::Next) => {
                self.selected = (self.selected + 1).min(self.names.len().saturating_sub(1));
                None
            }
            (Screen::List, Key::Prev) => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            (Screen::List, Key::Submit) => {
                self.names.get(self.selected).cloned().map(Request::Load)
            }
            (Screen::Preview, Key::Reload) => Some(Request::Load(self.name.clone())),
            (Screen::Preview, key @ (Key::Submit | Key::Connect | Key::Attest)) => {
                match self.resolved() {
                    Err(error) => {
                        self.error = Some(error);
                        None
                    }
                    Ok(profile) if profile.authorized_at_unix.is_none() && key != Key::Attest => {
                        self.error = Some(
                            "Press a to attest authorization and connect, or Esc to cancel".into(),
                        );
                        None
                    }
                    Ok(_) => Some(Request::Connect {
                        name: self.name.clone(),
                        profile: self.saved.clone().expect("resolved profile"),
                        attest: key == Key::Attest,
                    }),
                }
            }
            _ => None,
        };
        request.map(|request| {
            self.pending = true;
            Command::Profile(request)
        })
    }
}
