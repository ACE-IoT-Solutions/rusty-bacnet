//! Profile I/O at the coordinator boundary, before client construction.

use super::{store::Store, Overrides, Profile};
use std::time::{SystemTime, UNIX_EPOCH};

/// Pure UI requests. Connecting includes the exact document previewed by the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    List,
    Load(String),
    Save {
        name: String,
        profile: Profile,
        create: bool,
        previous: Option<Profile>,
    },
    Connect {
        name: String,
        profile: Profile,
        attest: bool,
    },
}

/// A successful filesystem operation, rendered by the pure UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    Catalog { names: Vec<String>, root: String },
    Loaded { name: String, profile: Profile },
}

/// Owns the store and command-line overrides for a profile-enabled session.
pub(crate) struct Service {
    pub(crate) store: Result<Store, String>,
    pub(crate) overrides: Overrides,
}

impl Service {
    pub(crate) fn new(overrides: Overrides) -> Self {
        Self {
            store: Store::discover(),
            overrides,
        }
    }
    pub(crate) fn catalog(&self) -> Result<Reply, String> {
        let store = self.store.as_ref().map_err(Clone::clone)?;
        Ok(Reply::Catalog {
            names: store.list()?,
            root: store.root().display().to_string(),
        })
    }
    pub(crate) fn load(&self, name: &str) -> Result<Reply, String> {
        let store = self.store.as_ref().map_err(Clone::clone)?;
        let profile = store.load(name)?;
        self.overrides.resolve(&profile)?;
        Ok(Reply::Loaded {
            name: name.into(),
            profile,
        })
    }
    pub(crate) fn save(
        &self,
        name: &str,
        profile: Profile,
        create: bool,
        previous: Option<Profile>,
    ) -> Result<Reply, String> {
        let store = self.store.as_ref().map_err(Clone::clone)?;
        self.overrides.resolve(&profile)?;
        if !create && previous.as_ref() != Some(&store.load(name)?) {
            return Err("profile changed on disk; reload before saving".into());
        }
        store.save(name, &profile, create)?;
        Ok(Reply::Loaded {
            name: name.into(),
            profile,
        })
    }
    /// The workspace of profile `name`, which connecting it created.
    pub(crate) fn workspace(&self, name: &str) -> Option<std::path::PathBuf> {
        self.store.as_ref().ok()?.workspace(name).ok()
    }
    /// Validate the displayed snapshot, persist authorization and prepare its
    /// workspace, then return the already resolved connection configuration.
    pub(crate) fn prepare(
        &self,
        name: &str,
        displayed: &Profile,
        attest: bool,
    ) -> Result<Profile, String> {
        let store = self.store.as_ref().map_err(Clone::clone)?;
        let mut profile = store.load(name)?;
        if profile != *displayed {
            return Err("profile changed on disk; reload and review it before connecting".into());
        }
        let mut resolved = self.overrides.resolve(&profile)?;
        if profile.authorized_at_unix.is_none() {
            if !attest {
                return Err("explicit authorization is required before first use".into());
            }
            let at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| format!("cannot timestamp authorization: {e}"))?
                .as_secs();
            profile.authorized_at_unix = Some(at);
            profile.authorized_safety = Some(profile.safety.clone());
            store.save(name, &profile, false)?;
            resolved.authorized_at_unix = Some(at);
            resolved.authorized_safety = profile.authorized_safety.clone();
        }
        store.workspace(name)?;
        Ok(resolved)
    }
}
