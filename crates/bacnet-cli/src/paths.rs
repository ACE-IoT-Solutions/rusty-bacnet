//! Where the CLI keeps its files: the platform config and data directories
//! from `directories`, never `$HOME` directly (Windows has no `HOME`).
//!
//! Resolution is pure: the environment and the platform directories are
//! passed in, so each OS's layout is testable without touching the real
//! environment. Nothing here creates a directory.

use directories::ProjectDirs;
#[cfg(any(feature = "tui", test))]
use std::ffi::OsString;
use std::path::PathBuf;

/// Environment variable that replaces the config directory, and with it
/// every profile and profile workspace.
#[cfg(any(feature = "tui", test))]
pub(crate) const CONFIG_DIR_ENV: &str = "BACNET_CONFIG_DIR";

/// File name of the interactive shell's line history in the data directory.
pub(crate) const HISTORY_FILE: &str = "shell_history";

/// This program's platform directories, if the OS reports a home directory.
pub(crate) fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "bacnet")
}

/// The config root: the override when set, else the platform config dir.
#[cfg(any(feature = "tui", test))]
pub(crate) fn config_root(
    env_override: Option<OsString>,
    dirs: Option<&ProjectDirs>,
) -> Result<PathBuf, String> {
    if let Some(root) = env_override {
        if root.is_empty() {
            return Err(format!("{CONFIG_DIR_ENV} must not be empty"));
        }
        return Ok(PathBuf::from(root));
    }
    dirs.map(|dirs| dirs.config_dir().to_owned())
        .ok_or_else(|| format!("cannot locate the config directory; set {CONFIG_DIR_ENV}"))
}

/// The config root of this process.
#[cfg(feature = "tui")]
pub(crate) fn discover_config_root() -> Result<PathBuf, String> {
    config_root(std::env::var_os(CONFIG_DIR_ENV), project_dirs().as_ref())
}

/// The shell history file in the platform data dir. `None` without a home
/// directory: then history is kept for the session only, never written to
/// the working directory.
pub(crate) fn history_file(dirs: Option<&ProjectDirs>) -> Option<PathBuf> {
    dirs.map(|dirs| dirs.data_dir().join(HISTORY_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn dirs() -> ProjectDirs {
        project_dirs().expect("CI runners have a home directory")
    }

    #[test]
    fn override_wins_and_must_not_be_empty() {
        let root = config_root(Some("/tmp/site-a".into()), Some(&dirs())).unwrap();
        assert_eq!(root, Path::new("/tmp/site-a"));
        assert_eq!(
            config_root(Some("relative".into()), None).unwrap(),
            Path::new("relative")
        );
        assert!(config_root(Some(OsString::new()), Some(&dirs()))
            .unwrap_err()
            .contains(CONFIG_DIR_ENV));
        assert!(config_root(None, None)
            .unwrap_err()
            .contains(CONFIG_DIR_ENV));
    }

    #[test]
    fn default_config_root_is_the_platform_config_dir() {
        let dirs = dirs();
        assert_eq!(config_root(None, Some(&dirs)).unwrap(), dirs.config_dir());
    }

    /// The platform layout, checked natively on each CI OS.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_uses_xdg_config_and_data_dirs() {
        let dirs = dirs();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let xdg = |var: &str, fallback: &str| {
            std::env::var_os(var)
                .filter(|v| Path::new(v).is_absolute())
                .map_or_else(|| home.join(fallback), PathBuf::from)
        };
        assert_eq!(
            config_root(None, Some(&dirs)).unwrap(),
            xdg("XDG_CONFIG_HOME", ".config").join("bacnet")
        );
        assert_eq!(
            history_file(Some(&dirs)).unwrap(),
            xdg("XDG_DATA_HOME", ".local/share")
                .join("bacnet")
                .join(HISTORY_FILE)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_uses_application_support() {
        let dirs = dirs();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let support = home.join("Library/Application Support/bacnet");
        assert_eq!(config_root(None, Some(&dirs)).unwrap(), support);
        assert_eq!(
            history_file(Some(&dirs)).unwrap(),
            support.join(HISTORY_FILE)
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_uses_roaming_app_data_not_home() {
        let dirs = dirs();
        let roaming = std::env::var_os("APPDATA").map(PathBuf::from).unwrap();
        assert_eq!(
            config_root(None, Some(&dirs)).unwrap(),
            roaming.join("bacnet").join("config")
        );
        let history = history_file(Some(&dirs)).unwrap();
        assert_eq!(
            history,
            roaming.join("bacnet").join("data").join(HISTORY_FILE)
        );
        // The old location was `%HOME%\.bacnet_history`, or the working
        // directory when HOME was unset, which is the usual case on Windows.
        assert!(history.is_absolute());
        assert_ne!(history.file_name().unwrap(), ".bacnet_history");
    }

    #[test]
    fn no_home_means_no_history_file() {
        assert_eq!(history_file(None), None);
        let history = history_file(Some(&dirs())).unwrap();
        assert!(history.is_absolute());
        assert!(history.starts_with(dirs().data_dir()));
    }
}
