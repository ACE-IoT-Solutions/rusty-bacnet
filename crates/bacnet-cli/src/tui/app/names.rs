//! Object and device names for the browser's object list and the device
//! table (#1896).
//!
//! The [`NameCache`] holds every name the session has learned, keyed by
//! device and object (a device's own name under its Device object). Names
//! that arrive free fill it first: the device card, an object read in the
//! browser, a watch row's first read and every I-Have. For the rest, each
//! tick asks for the rows the last frame showed without a name, and only
//! those: the views mark which rows those were, and only when they had
//! room for a name. One command carries every device's ask, so a tick
//! never crowds the worker's queues.
//!
//! What each device was last asked for, less what has arrived since, is
//! kept, so a tick that shows the same rows (in any order) sends nothing;
//! when the rows change, the new set replaces the old one in the worker,
//! which drops rows scrolled off before their turn. A name the device
//! refused is kept as [`Name::Unavailable`] and not asked for again until
//! the device is read afresh (its card arrives). A name it didn't answer
//! for is [`Name::Unreachable`] until the device's next I-Am.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use super::objects::ListRow;
use super::watch::Screen;
use super::{App, Link};
use crate::tui::format::clean_name;
use crate::tui::message::Command;
use crate::tui::point::{NameAsk, ReadName, ReqId};

/// What the session knows of one object's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Name {
    /// Its name.
    Known(String),
    /// The device refused it.
    Unavailable,
    /// The device didn't answer; asked again after its next I-Am.
    Unreachable,
}

static UNREACHABLE: Name = Name::Unreachable;

/// What one device was last asked for.
#[derive(Debug, Default)]
struct Ask {
    /// The request that carried it.
    req: ReqId,
    /// The objects asked for that haven't arrived.
    objects: HashSet<ObjectIdentifier>,
    /// It reached the worker, as far as the UI knows; a refusal clears it.
    sent: bool,
    /// The next ask must tell the worker to read unreachable names again.
    retry: bool,
    /// The ask that went carried `retry`, to restore if it is refused.
    retried: bool,
}

/// The session's names, and what each device was last asked for.
#[derive(Debug, Default)]
pub(crate) struct NameCache {
    names: HashMap<(u32, ObjectIdentifier), Name>,
    /// Per device, the names it didn't answer for.
    stalled: HashMap<u32, HashSet<ObjectIdentifier>>,
    asked: BTreeMap<u32, Ask>,
}

impl NameCache {
    /// What is known of `object`'s name on `device`.
    pub(crate) fn get(&self, device: u32, object: ObjectIdentifier) -> Option<&Name> {
        match self.names.get(&(device, object)) {
            Some(name) => Some(name),
            None => self
                .stalled
                .get(&device)
                .is_some_and(|stalled| stalled.contains(&object))
                .then_some(&UNREACHABLE),
        }
    }

    /// Learn a name that came free.
    pub(crate) fn learn_name(&mut self, device: u32, object: ObjectIdentifier, name: String) {
        self.learn(device, object, ReadName::Named(name));
    }

    /// Learn how `object`'s name read went. A name once known isn't lost to
    /// a later failure, nor a refusal to a timeout. Names are remote text,
    /// so they are cleaned first ([`clean_name`]).
    pub(crate) fn learn(&mut self, device: u32, object: ObjectIdentifier, read: ReadName) {
        if let Some(ask) = self.asked.get_mut(&device) {
            ask.objects.remove(&object);
        }
        let known = self.names.get(&(device, object));
        let name = match read {
            ReadName::Named(name) => Name::Known(clean_name(&name)),
            ReadName::Refused if !matches!(known, Some(Name::Known(_))) => Name::Unavailable,
            ReadName::Unreachable if known.is_none() => {
                self.stalled.entry(device).or_default().insert(object);
                return;
            }
            ReadName::Refused | ReadName::Unreachable => return,
        };
        self.names.insert((device, object), name);
        if let Some(stalled) = self.stalled.get_mut(&device) {
            stalled.remove(&object);
        }
    }

    /// `device` sent an I-Am: the names it didn't answer for are asked
    /// for again, and the worker told to read them.
    pub(crate) fn answered(&mut self, device: u32) {
        if self.stalled.remove(&device).is_some_and(|s| !s.is_empty()) {
            self.asked.entry(device).or_default().retry = true;
        }
    }

    /// `device` was read afresh (its card arrived): every name it refused
    /// or didn't answer for is asked for again. The worker forgot them
    /// when it took the read.
    pub(crate) fn forget_failures(&mut self, device: u32) {
        self.names
            .retain(|(of, _), name| *of != device || *name != Name::Unavailable);
        self.stalled.remove(&device);
    }

    /// Request `req` didn't reach the worker: what it asked, withdrawals
    /// included, is asked again on the next tick. Returns whether it was a
    /// name request.
    pub(crate) fn refused(&mut self, req: ReqId) -> bool {
        let mut found = false;
        for ask in self
            .asked
            .values_mut()
            .filter(|ask| ask.req == req && ask.sent)
        {
            ask.sent = false;
            ask.retry |= ask.retried;
            found = true;
        }
        found
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

    /// Each tick: one command asking each device for the names on screen
    /// it hasn't been asked for, and withdrawing the ones that left it.
    pub(super) fn ask_names(&mut self, out: &mut Vec<Command>) {
        let wanted = if self.names_live() {
            self.names_on_screen()
        } else {
            BTreeMap::new()
        };
        let devices: BTreeSet<u32> = self
            .names
            .asked
            .keys()
            .chain(wanted.keys())
            .copied()
            .collect();
        let mut asks = Vec::new();
        for device in devices {
            let now = wanted.get(&device).map_or(&[][..], Vec::as_slice);
            let unchanged = match self.names.asked.get(&device) {
                Some(ask) => {
                    ask.sent
                        && !ask.retry
                        && ask.objects.len() == now.len()
                        && now.iter().all(|object| ask.objects.contains(object))
                }
                None => now.is_empty(),
            };
            if !unchanged {
                let retry = self.names.asked.get(&device).is_some_and(|ask| ask.retry);
                asks.push(NameAsk {
                    device,
                    objects: now.to_vec(),
                    retry,
                });
            }
        }
        if asks.is_empty() {
            return;
        }
        let req = self.next_req();
        for ask in &asks {
            let entry = Ask {
                req,
                objects: ask.objects.iter().copied().collect(),
                sent: true,
                retry: false,
                retried: ask.retry,
            };
            self.names.asked.insert(ask.device, entry);
        }
        out.push(Command::Names { req, asks });
    }
}
