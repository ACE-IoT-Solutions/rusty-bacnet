//! Pre-connect checks that need the filesystem: the files a profile names
//! must be readable before anything is sent. Run at the coordinator boundary
//! (never in the pure UI), both for the preview and again just before a
//! connection attempt, which an error refuses.

use super::{Profile, Sc};
use std::fs::File;
use std::path::Path;

/// What the connection screen shows under the resolved settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Checks {
    /// Problems that refuse a connection.
    pub(crate) errors: Vec<String>,
    /// Problems worth fixing that still allow one.
    pub(crate) warnings: Vec<String>,
}

impl Checks {
    /// The first error, as a refusal.
    pub(crate) fn refusal(&self) -> Result<(), String> {
        match self.errors.first() {
            Some(error) => Err(format!("validation failed: {error}")),
            None => Ok(()),
        }
    }
}

/// Inspect the files of an already resolved profile.
pub(crate) fn inspect(profile: &Profile) -> Checks {
    let mut checks = Checks::default();
    if let Some(sc) = &profile.sc {
        inspect_sc(sc, &mut checks);
    }
    checks
}

fn inspect_sc(sc: &Sc, checks: &mut Checks) {
    for (label, path) in [
        ("CA file", &sc.ca),
        ("Certificate file", &sc.cert),
        ("Key file", &sc.key),
    ] {
        if let Err(error) = readable(path) {
            checks
                .errors
                .push(format!("{label} {} {error}", path.display()));
        }
    }
    #[cfg(unix)]
    if let Some(warning) = key_exposure(&sc.key) {
        checks.warnings.push(warning);
    }
}

/// Open the file for reading; a directory or an unreadable file fails.
fn readable(path: &Path) -> Result<(), String> {
    let file = File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "does not exist".to_owned(),
        std::io::ErrorKind::PermissionDenied => "is not readable".to_owned(),
        _ => format!("cannot be opened: {e}"),
    })?;
    match file.metadata() {
        Ok(meta) if meta.is_file() => Ok(()),
        Ok(_) => Err("is not a regular file".into()),
        Err(e) => Err(format!("cannot be inspected: {e}")),
    }
}

/// A private key that the group or other users can read.
#[cfg(unix)]
fn key_exposure(path: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).ok()?.permissions().mode() & 0o777;
    (mode & 0o077 != 0).then(|| {
        format!(
            "Key file {} is readable by group or others (mode {mode:o}); chmod 600 it.",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc_profile(dir: &Path) -> Profile {
        let mut profile = Profile::sc_default();
        let sc = profile.sc.as_mut().unwrap();
        sc.hub_url = "wss://127.0.0.1:1".into();
        for (name, slot) in [
            ("ca.pem", &mut sc.ca),
            ("cert.pem", &mut sc.cert),
            ("key.pem", &mut sc.key),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, "placeholder").unwrap();
            *slot = path;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sc.key, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        profile
    }

    #[test]
    fn readable_files_pass_and_each_missing_file_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let mut profile = sc_profile(dir.path());
        assert_eq!(inspect(&profile), Checks::default());
        assert_eq!(inspect(&Profile::default()), Checks::default());
        let sc = profile.sc.as_mut().unwrap();
        sc.ca = dir.path().join("missing-ca.pem");
        sc.cert = dir.path().to_owned();
        let checks = inspect(&profile);
        assert_eq!(checks.errors.len(), 2, "{checks:?}");
        assert!(checks.errors[0].contains("missing-ca.pem"));
        assert!(checks.errors[0].contains("does not exist"));
        // A directory: refused on open (Windows) or as not a regular file.
        assert!(checks.errors[1].starts_with("Certificate file"));
        assert!(checks.refusal().unwrap_err().contains("missing-ca.pem"));
    }

    #[cfg(unix)]
    #[test]
    fn group_or_world_readable_keys_warn_without_refusing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let profile = sc_profile(dir.path());
        let key = &profile.sc.as_ref().unwrap().key;
        for mode in [0o640, 0o604, 0o644] {
            std::fs::set_permissions(key, std::fs::Permissions::from_mode(mode)).unwrap();
            let checks = inspect(&profile);
            assert!(checks.errors.is_empty());
            assert_eq!(checks.warnings.len(), 1, "{mode:o}");
            assert!(checks.warnings[0].contains("key.pem"));
            assert!(checks.refusal().is_ok());
        }
        std::fs::set_permissions(key, std::fs::Permissions::from_mode(0o400)).unwrap();
        assert!(inspect(&profile).warnings.is_empty());
    }
}
