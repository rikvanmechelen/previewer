//! Persisted UI preferences. JSON file at `$XDG_CONFIG_HOME/previewer/
//! settings.json` (falling back to `$HOME/.config/previewer/settings.json`).
//!
//! Conservative read path: any I/O or parse error returns
//! [`Settings::default()`] rather than propagating, since a corrupt or
//! missing settings file should never block the app from launching.
//! Writes are explicit (the caller decides when to persist) so we don't
//! flush to disk on every internal mutation.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Persisted UI preferences. Add fields with `#[serde(default)]` so old
/// settings files keep loading after we extend the schema.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Whether the page-thumbnails sidebar should be visible on launch.
    pub show_sidebar: bool,
}

impl Settings {
    /// Load from the canonical settings path. Returns
    /// [`Settings::default()`] if the file is missing, unreadable, or
    /// malformed.
    pub fn load() -> Self {
        let Some(path) = settings_path() else {
            return Self::default();
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    /// Persist to the canonical path, creating parent directories as
    /// needed. Caller decides when to flush.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = settings_path() else {
            return Err(std::io::Error::other("no config-base directory available"));
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).unwrap_or_default();
        std::fs::write(path, json)
    }

    /// Variant of [`load`] with an explicit base directory — used by
    /// tests so they don't touch the real `~/.config`.
    #[doc(hidden)]
    pub fn load_from(dir: &std::path::Path) -> Self {
        let Ok(bytes) = std::fs::read(dir.join("settings.json")) else {
            return Self::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    /// Variant of [`save`] with an explicit directory.
    #[doc(hidden)]
    pub fn save_to(&self, dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_string_pretty(self).unwrap_or_default();
        std::fs::write(dir.join("settings.json"), json)
    }
}

fn settings_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("previewer").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_round_trips() {
        let s = Settings::default();
        let json = serde_json::to_string(&s).unwrap();
        let parsed: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, parsed);
    }

    #[test]
    fn save_and_load_via_explicit_dir() {
        let dir = TempDir::new().unwrap();
        let s = Settings { show_sidebar: true };
        s.save_to(dir.path()).unwrap();

        let parsed = Settings::load_from(dir.path());
        assert_eq!(parsed, s);
    }

    #[test]
    fn missing_file_yields_default() {
        let dir = TempDir::new().unwrap();
        let parsed = Settings::load_from(dir.path());
        assert_eq!(parsed, Settings::default());
    }

    #[test]
    fn corrupt_file_yields_default() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("settings.json"), b"not json").unwrap();
        let parsed = Settings::load_from(dir.path());
        assert_eq!(parsed, Settings::default());
    }

    #[test]
    fn legacy_file_without_show_sidebar_field_loads() {
        // Forward-compat: if we ever release a settings file with no
        // `show_sidebar` (e.g. user pre-loaded their own), it must still
        // parse rather than blowing up.
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("settings.json"), b"{}").unwrap();
        let parsed = Settings::load_from(dir.path());
        assert_eq!(parsed, Settings::default());
    }
}
