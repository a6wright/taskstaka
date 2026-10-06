//! Machine-level settings: `~/.config/taskstaka/settings.json`
//! (`$XDG_CONFIG_HOME` honoured, `$TASKSTAKA_CONFIG_DIR` overrides both so
//! tests never touch the real one).
//!
//! They live outside the notes directory on purpose: where the notes are
//! cannot be stored inside them, and a popup preference belongs to this
//! machine, not to notes you may sync. Read at the moment they matter, so a
//! change from the web, the CLI or by hand applies without a restart (the
//! notes directory excepted: the daemon is bound to one).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const CONFIG_DIR_ENV: &str = "TASKSTAKA_CONFIG_DIR";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Desktop notifications. Off still computes nudges and shows them in the
    /// panel; it only stops the popups.
    pub notifications: bool,
    /// Where the notes live. `None` means `~/todo`. `--dir` and
    /// `$TASKSTAKA_DIR` still win over it.
    pub dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            notifications: true,
            dir: None,
        }
    }
}

impl Settings {
    /// The directory holding `settings.json`.
    pub fn config_dir() -> PathBuf {
        if let Some(d) = std::env::var_os(CONFIG_DIR_ENV).filter(|d| !d.is_empty()) {
            return PathBuf::from(d);
        }
        if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
            return PathBuf::from(x).join("taskstaka");
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config")
            .join("taskstaka")
    }

    pub fn path_in(config_dir: &Path) -> PathBuf {
        config_dir.join("settings.json")
    }

    /// Missing or unreadable means defaults: a broken settings file must never
    /// stop the daemon.
    pub fn load_from(config_dir: &Path) -> Settings {
        fs::read_to_string(Self::path_in(config_dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn load() -> Settings {
        Self::load_from(&Self::config_dir())
    }

    pub fn save_to(&self, config_dir: &Path) -> io::Result<()> {
        fs::create_dir_all(config_dir)?;
        let path = Self::path_in(config_dir);
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self)? + "\n")?;
        fs::rename(tmp, path)
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(&Self::config_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings::load_from(dir.path());
        assert!(s.notifications);
        assert_eq!(s.dir, None);
        Settings {
            notifications: false,
            dir: Some("/tmp/notes".into()),
        }
        .save_to(dir.path())
        .unwrap();
        let s = Settings::load_from(dir.path());
        assert!(!s.notifications);
        assert_eq!(s.dir, Some(PathBuf::from("/tmp/notes")));
    }

    #[test]
    fn a_broken_file_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(Settings::path_in(dir.path()), "{not json").unwrap();
        assert_eq!(Settings::load_from(dir.path()), Settings::default());
    }

    #[test]
    fn a_partial_file_keeps_the_other_defaults() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(Settings::path_in(dir.path()), r#"{"dir": "/x"}"#).unwrap();
        let s = Settings::load_from(dir.path());
        assert!(s.notifications);
        assert_eq!(s.dir, Some(PathBuf::from("/x")));
    }
}
