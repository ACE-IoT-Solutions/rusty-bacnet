//! Object and device names for the browser's object list and the device
//! table (#1896).
//!
//! The [`NameCache`] holds every name the session has learned, keyed by
//! device and object (a device's own name under its Device object). Names
//! that arrive free fill it first: the device card, an object read in the
//! browser, a watch row's first read and every I-Have. For the rest, each
//! tick asks for the rows the last frame showed without a name, and only
//! those: the views mark which rows those were, and only when they had
//! room for a name.
//!
//! What a device was last asked for is kept, so a tick that shows the same
//! rows sends nothing; when the rows change, the new set replaces the old
//! one in the worker, which drops rows scrolled off before their turn. A
//! name the device couldn't give is kept as [`Name::Unavailable`] and not
//! asked for again, until `r` reads the device afresh.

use std::collections::{BTreeMap, HashMap};

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use super::objects::ListRow;
use super::watch::Screen;
use super::{App, Link};
use crate::tui::format::clean_name;
use crate::tui::message::Command;
use crate::tui::point::{BrowseRead, ReqId};

/// What the session knows of one object's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Name {
    /// Its name.
    Known(String),
    /// The device couldn't give it.
    Unavailable,
}

/// The session's names, and what each device was last asked for.
#[derive(Debug, Default)]
pub(crate) struct NameCache {
    names: HashMap<(u32, ObjectIdentifier), Name>,
    /// Per device: the request, and the objects it asked for.
    asked: BTreeMap<u32, (ReqId, Vec<ObjectIdentifier>)>,
}

impl NameCache {
    /// What is known of `object`'s name on `device`.
    pub(crate) fn get(&self, device: u32, object: ObjectIdentifier) -> Option<&Name> {
        self.names.get(&(device, object))
    }

    /// Learn `object`'s name, or that the device couldn't give it. A name
    /// once known isn't lost to a later failure. Names are remote text, so
    /// they are cleaned first ([`clean_name`]).
    pub(crate) fn learn(&mut self, device: u32, object: ObjectIdentifier, name: Option<String>) {
        let name = match name {
            Some(name) => Name::Known(clean_name(&name)),
            None if self.get(device, object).is_some() => return,
            None => Name::Unavailable,
        };
        self.names.insert((device, object), name);
    }

    /// Let `device`'s unavailable names be asked for again.
    pub(crate) fn forget_unavailable(&mut self, device: u32) {
        self.names
            .retain(|(of, _), name| *of != device || *name != Name::Unavailable);
    }

    /// Request `req` didn't reach the worker: ask again on the next tick.
    /// Returns whether it was a name request.
    pub(crate) fn refused(&mut self, req: ReqId) -> bool {
        let before = self.asked.len();
        self.asked.retain(|_, (asked, _)| *asked != req);
        self.asked.len() != before
    }
}

/// The Device object of `instance`.
pub(crate) fn device_object(instance: u32) -> Option<ObjectIdentifier> {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).ok()
}

impl App {
    /// True while names can be asked for: connected, and not passive.
    pub(crate) fn names_live(&self) -> bool {
        matches!(self.link, Link::Up { .. }) && !self.safety.passive
    }

    /// The rows the last frame showed without a name, by device.
    fn names_on_screen(&self) -> BTreeMap<u32, Vec<ObjectIdentifier>> {
        let mut wanted: BTreeMap<u32, Vec<ObjectIdentifier>> = BTreeMap::new();
        let mut want = |device: u32, object: ObjectIdentifier| {
            if self.names.get(device, object).is_none() {
                wanted.entry(device).or_default().push(object);
            }
        };
        let devices = &self.devices;
        if self.screen == Screen::Main && devices.named.get() {
            let rows = devices.visible_window(devices.offset.get(), devices.page.get());
            for row in rows {
                if let Some(object) = device_object(row.instance) {
                    want(row.instance, object);
                }
            }
        }
        if let Some(browser) = self.shown_browser().filter(|b| b.objects.named.get()) {
            let objects = &browser.objects;
            let rows = objects.rows().iter().skip(objects.offset.get());
            for row in rows.take(objects.page.get()) {
                if let ListRow::Object(object) = row {
                    want(browser.device, *object);
                }
            }
        }
        wanted
    }

    /// Each tick: ask each device for the names on screen it hasn't been
    /// asked for, and stop asking for the ones that left the screen.
    pub(super) fn ask_names(&mut self, out: &mut Vec<Command>) {
        let wanted = if self.names_live() {
            self.names_on_screen()
        } else {
            BTreeMap::new()
        };
        let mut devices: Vec<u32> = self.names.asked.keys().copied().collect();
        devices.extend(wanted.keys().filter(|d| !self.names.asked.contains_key(d)));
        for device in devices {
            let now = wanted.get(&device).cloned().unwrap_or_default();
            // Still being read: what was asked, less what has arrived.
            let pending: Vec<ObjectIdentifier> = self
                .names
                .asked
                .get(&device)
                .map(|(_, asked)| {
                    let unknown =
                        |object: &&ObjectIdentifier| self.names.get(device, **object).is_none();
                    asked.iter().filter(unknown).copied().collect()
                })
                .unwrap_or_default();
            if now == pending {
                if now.is_empty() {
                    self.names.asked.remove(&device);
                }
                continue;
            }
            let req = self.next_req();
            if now.is_empty() {
                self.names.asked.remove(&device);
            } else {
                self.names.asked.insert(device, (req, now.clone()));
            }
            out.push(Command::Browse {
                req,
                device,
                read: BrowseRead::Names(now),
            });
        }
    }
}
