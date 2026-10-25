//! Interface preferences of the desktop app that upstream's settings do not
//! carry: how much the interface moves.
//!
//! The file sits beside the other preferences (`~/.cheaprouter`) and is shared
//! by debug and release builds: it holds no ids, only choices the person made,
//! and the same person expects them in both.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::brand;
use crate::global_config::atomic_write_private;

pub const FILE_NAME: &str = "ui-prefs.json";

/// Whether non-essential motion is reduced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReduceMotion {
    /// Follow the operating system's setting.
    #[default]
    System,
    /// Always reduce motion.
    On,
    /// Never reduce motion, whatever the system says.
    Off,
}

impl ReduceMotion {
    pub const ALL: [Self; 3] = [Self::System, Self::On, Self::Off];
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiPrefs {
    #[serde(default)]
    pub reduce_motion: ReduceMotion,
    /// Hold every animation still while the window is in the background.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pause_motion_in_background: bool,
}

impl UiPrefs {
    /// Whether motion is reduced when the preference alone decides it, given
    /// whether the window is the active one. `None` leaves it to the system.
    pub fn fixed_reduce_motion(&self, window_active: bool) -> Option<bool> {
        if self.pause_motion_in_background && !window_active {
            return Some(true);
        }
        match self.reduce_motion {
            ReduceMotion::System => None,
            ReduceMotion::On => Some(true),
            ReduceMotion::Off => Some(false),
        }
    }
}

pub fn path() -> Option<PathBuf> {
    brand::data_dir().map(|dir| dir.join(FILE_NAME))
}

/// The preferences on disk; absent or unreadable means the defaults.
pub fn load() -> UiPrefs {
    path().map(|path| load_from(&path)).unwrap_or_default()
}

pub fn load_from(path: &Path) -> UiPrefs {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(prefs: &UiPrefs) -> Result<()> {
    let path = path().ok_or_else(|| anyhow!("could not locate the home directory"))?;
    save_to(&path, prefs)
}

pub fn save_to(path: &Path, prefs: &UiPrefs) -> Result<()> {
    let mut encoded =
        serde_json::to_string_pretty(prefs).context("could not encode interface preferences")?;
    encoded.push('\n');
    atomic_write_private(path, encoded.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choice_decides_unless_the_window_is_paused_in_the_background() {
        let mut prefs = UiPrefs::default();
        assert_eq!(prefs.fixed_reduce_motion(true), None);
        assert_eq!(prefs.fixed_reduce_motion(false), None);

        prefs.reduce_motion = ReduceMotion::On;
        assert_eq!(prefs.fixed_reduce_motion(true), Some(true));

        prefs.reduce_motion = ReduceMotion::Off;
        assert_eq!(prefs.fixed_reduce_motion(true), Some(false));
        assert_eq!(prefs.fixed_reduce_motion(false), Some(false));

        prefs.pause_motion_in_background = true;
        assert_eq!(prefs.fixed_reduce_motion(true), Some(false));
        assert_eq!(prefs.fixed_reduce_motion(false), Some(true));

        prefs.reduce_motion = ReduceMotion::System;
        assert_eq!(prefs.fixed_reduce_motion(true), None);
        assert_eq!(prefs.fixed_reduce_motion(false), Some(true));
    }

    #[test]
    fn preferences_round_trip_and_missing_fields_mean_defaults() {
        let dir = std::env::temp_dir().join(format!("ui-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);

        assert_eq!(load_from(&path), UiPrefs::default());

        let prefs = UiPrefs {
            reduce_motion: ReduceMotion::Off,
            pause_motion_in_background: true,
        };
        save_to(&path, &prefs).unwrap();
        assert_eq!(load_from(&path), prefs);

        std::fs::write(&path, "{}").unwrap();
        assert_eq!(load_from(&path), UiPrefs::default());
        std::fs::write(&path, r#"{"reduce_motion":"on"}"#).unwrap();
        assert_eq!(load_from(&path).reduce_motion, ReduceMotion::On);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
