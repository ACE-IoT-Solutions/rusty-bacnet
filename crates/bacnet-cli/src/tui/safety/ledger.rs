//! The session ledger: every change still in effect, with the pre-image
//! that undoes it. It is saved to `ledger.json` in the profile workspace
//! after each change, atomically, so a crash leaves it behind for the next
//! launch to list.
//!
//! An entry is written before its request goes out (state `sending`), so
//! even a kill mid-request leaves a record. Repeated writes to one slot or
//! property keep the first pre-image; a write that puts back the pre-image
//! (relinquishing a slot that was empty, say) closes the entry.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use serde::{Deserialize, Serialize};

use super::{Planned, Stored, WriteSpec, NULL};

/// The ledger's file in the workspace.
pub(crate) const FILE: &str = "ledger.json";

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Where an entry stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum State {
    /// The request is out; no answer yet.
    Sending,
    /// Acknowledged.
    Applied,
    /// No answer came, so it may be in effect.
    Unknown,
}

/// One change in effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Entry {
    /// Unique within the file.
    pub(crate) id: u64,
    /// The process run that made it.
    pub(crate) run: String,
    /// When it was first written, UTC.
    pub(crate) at: String,
    /// Device instance.
    pub(crate) device: u32,
    /// Object type, raw.
    pub(crate) object_type: u32,
    /// Object instance.
    pub(crate) instance: u32,
    /// Property, raw.
    pub(crate) property: u32,
    /// Array index.
    #[serde(default)]
    pub(crate) index: Option<u32>,
    /// The priority slot, for a commandable write.
    #[serde(default)]
    pub(crate) priority: Option<u8>,
    /// Written to a priority slot.
    pub(crate) commandable: bool,
    /// What undoes it: the slot's old value (`None`: it was NULL) or the
    /// property's old value (`None`: unknown, so it can't be undone).
    pub(crate) before: Option<Stored>,
    /// What was written last.
    pub(crate) written: Stored,
    /// Where it stands.
    pub(crate) state: State,
    /// An earlier session chose to keep it on exit.
    #[serde(default)]
    pub(crate) kept: bool,
}

impl Entry {
    /// The object.
    pub(crate) fn object(&self) -> Option<ObjectIdentifier> {
        ObjectIdentifier::new(ObjectType::from_raw(self.object_type), self.instance).ok()
    }

    /// The property.
    pub(crate) fn property(&self) -> PropertyIdentifier {
        PropertyIdentifier::from_raw(self.property)
    }

    /// `device 100 analog-output 1 present-value @8`.
    pub(crate) fn label(&self) -> String {
        match self.object() {
            Some(object) => super::label(
                self.device,
                object,
                self.property(),
                self.index,
                self.priority,
            ),
            None => format!("device {} object {}", self.device, self.object_type),
        }
    }

    /// The value and priority that undo it: the old slot value or NULL at
    /// the slot, or the old property value. `None` when the old value is
    /// unknown.
    pub(crate) fn undo(&self) -> Option<Stored> {
        match (&self.before, self.commandable) {
            (Some(before), _) => Some(before.clone()),
            (None, true) => Some(Stored::null()),
            (None, false) => None,
        }
    }

    fn same_target(&self, spec: &WriteSpec, priority: Option<u8>) -> bool {
        self.device == spec.device
            && self.object_type == spec.object.object_type().to_raw()
            && self.instance == spec.object.instance_number()
            && self.property == spec.property.to_raw()
            && self.index == spec.index
            && self.priority == priority
    }

    /// The device is back at the pre-image.
    fn undone(&self) -> bool {
        match &self.before {
            Some(before) => before.bytes == self.written.bytes,
            None => self.commandable && self.written.bytes == NULL,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct LedgerFile {
    version: u32,
    entries: Vec<Entry>,
}

/// The ledger and its file.
#[derive(Debug)]
pub(crate) struct Ledger {
    path: PathBuf,
    entries: Vec<Entry>,
}

impl Ledger {
    /// Load the ledger in `workspace`; none yet is an empty one.
    pub(crate) fn open(workspace: &Path) -> Result<Self, String> {
        let path = workspace.join(FILE);
        let entries = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let file: LedgerFile = serde_json::from_str(&text)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                file.entries
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        Ok(Self { path, entries })
    }

    /// The entries, oldest first.
    pub(crate) fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// True when an entry covers `spec`'s slot at `priority`, or its
    /// property.
    pub(crate) fn holds(&self, spec: &WriteSpec, priority: Option<u8>) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.same_target(spec, priority))
    }

    /// Record `planned` as sending, merging into an entry for the same
    /// target (whose first pre-image stays). Returns the entry's id.
    pub(crate) fn begin(&mut self, planned: &Planned, run: &str, at: &str) -> u64 {
        let (spec, priority) = (&planned.spec, planned.priority);
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.same_target(spec, priority))
        {
            entry.written = planned.value.clone();
            entry.state = State::Sending;
            return entry.id;
        }
        let id = self.entries.iter().map(|e| e.id).max().map_or(1, |n| n + 1);
        let spec = &planned.spec;
        self.entries.push(Entry {
            id,
            run: run.into(),
            at: at.into(),
            device: spec.device,
            object_type: spec.object.object_type().to_raw(),
            instance: spec.object.instance_number(),
            property: spec.property.to_raw(),
            index: spec.index,
            priority: planned.priority,
            commandable: planned.commandable,
            before: planned.before.clone(),
            written: planned.value.clone(),
            state: State::Sending,
            kept: false,
        });
        id
    }

    /// Settle entry `id`: applied, refused (put `backup`'s version of it
    /// back, or drop it if it is new), or unknown.
    pub(crate) fn finish(&mut self, id: u64, applied: Option<bool>, backup: &[Entry]) {
        match applied {
            Some(false) => {
                let old = backup.iter().find(|e| e.id == id).cloned();
                self.entries.retain(|e| e.id != id);
                if let Some(old) = old {
                    self.entries.push(old);
                    self.entries.sort_by_key(|e| e.id);
                }
            }
            Some(true) => {
                if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
                    entry.state = State::Applied;
                }
                self.entries.retain(|e| e.id != id || !e.undone());
            }
            None => {
                if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
                    entry.state = State::Unknown;
                }
            }
        }
    }

    /// Drop entry `id`, once undone.
    pub(crate) fn remove(&mut self, id: u64) {
        self.entries.retain(|e| e.id != id);
    }

    /// Mark every entry as kept on exit.
    pub(crate) fn keep_all(&mut self) {
        for entry in &mut self.entries {
            entry.kept = true;
        }
    }

    /// Write the file: a complete, synced temporary file renamed over it.
    pub(crate) fn save(&self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&LedgerFile {
            version: 1,
            entries: self.entries.clone(),
        })
        .map_err(|e| format!("cannot encode the ledger: {e}"))?;
        let directory = self.path.parent().unwrap_or(Path::new("."));
        let temp = directory.join(format!(
            ".ledger-{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        crate::tui::profile::store::publish(&temp, &self.path, &text, false)
            .map_err(|e| format!("cannot save {}: {e}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use bacnet_types::primitives::PropertyValue;

    use super::super::{Effect, Gate, Occupancy, Tier};
    use super::*;

    fn stored(value: f32) -> Stored {
        let mut buf = bytes::BytesMut::new();
        bacnet_encoding::primitives::encode_property_value(&mut buf, &PropertyValue::Real(value))
            .unwrap();
        Stored {
            bytes: buf.to_vec(),
            text: value.to_string(),
        }
    }

    fn planned(value: Stored, before: Option<Stored>) -> Planned {
        Planned {
            spec: WriteSpec {
                device: 100,
                object: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                property: PropertyIdentifier::PRESENT_VALUE,
                index: None,
                value: None,
                priority: Some(8),
            },
            priority: Some(8),
            value,
            commandable: true,
            before,
            occupancy: Occupancy::Empty,
            effect: Effect::NotPresentValue,
            out_of_service: None,
            notes: Vec::new(),
            gate: Gate::Allowed(Tier::T2),
        }
    }

    #[test]
    fn writes_merge_keep_the_first_pre_image_and_close_on_relinquish() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::open(dir.path()).unwrap();
        let backup = ledger.entries().to_vec();
        let id = ledger.begin(&planned(stored(42.0), None), "run-a", "t");
        ledger.finish(id, Some(true), &backup);
        let backup = ledger.entries().to_vec();
        let again = ledger.begin(&planned(stored(43.0), Some(stored(42.0))), "run-a", "t");
        assert_eq!(again, id, "merged");
        ledger.finish(again, Some(true), &backup);
        assert_eq!(ledger.entries()[0].before, None, "the first pre-image");
        assert_eq!(ledger.entries()[0].undo(), Some(Stored::null()));
        ledger.save().unwrap();
        let reloaded = Ledger::open(dir.path()).unwrap();
        assert_eq!(reloaded.entries(), ledger.entries());
        // A refused write leaves the entry as it was.
        let backup = ledger.entries().to_vec();
        let id = ledger.begin(&planned(stored(44.0), None), "run-a", "t");
        ledger.finish(id, Some(false), &backup);
        assert_eq!(ledger.entries(), backup.as_slice());
        // Relinquishing the slot puts back its pre-image (NULL).
        let id = ledger.begin(&planned(Stored::null(), Some(stored(43.0))), "run-a", "t");
        ledger.finish(id, Some(true), &backup);
        assert!(ledger.entries().is_empty());
    }

    #[test]
    fn an_unanswered_write_stays_as_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::open(dir.path()).unwrap();
        let id = ledger.begin(&planned(stored(42.0), None), "run-a", "t");
        assert_eq!(ledger.entries()[0].state, State::Sending);
        ledger.finish(id, None, &[]);
        assert_eq!(ledger.entries()[0].state, State::Unknown);
    }
}
