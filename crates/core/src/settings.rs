//! User settings that outlive a daemon restart and apply without one:
//! `.taskstaka/settings.json`. Read at the moment they matter, so flipping a
//! switch from the web, the CLI or by hand takes effect immediately.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Desktop notifications. Off still computes nudges and shows them in the
    /// panel; it only stops the popups.
    pub notifications: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            notifications: true,
        }
    }
}

impl Settings {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(crate::store::META_DIR).join("settings.json")
    }

    /// Missing or unreadable means defaults: a broken settings file must never
    /// stop the daemon.
    pub fn load(dir: &Path) -> Settings {
        fs::read_to_string(Self::path(dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> io::Result<()> {
        let path = Self::path(dir);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self)? + "\n")?;
        fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_on_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Settings::load(dir.path()).notifications);
        Settings {
            notifications: false,
        }
        .save(dir.path())
        .unwrap();
        assert!(!Settings::load(dir.path()).notifications);
    }

    #[test]
    fn a_broken_file_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(crate::store::META_DIR)).unwrap();
        fs::write(Settings::path(dir.path()), "{not json").unwrap();
        assert!(Settings::load(dir.path()).notifications);
    }
}
