use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::computer_use::ComputerAppGrant;
use crate::model::ProviderKind;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(default)]
pub struct DaemonSettings {
    pub computer_use_enabled: bool,
    pub computer_use_allowed_apps: Vec<ComputerAppGrant>,
    pub disabled_providers: Vec<ProviderKind>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub provider_binary_overrides: HashMap<ProviderKind, String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for DaemonSettings {
    fn default() -> Self {
        Self {
            computer_use_enabled: false,
            computer_use_allowed_apps: Vec::new(),
            disabled_providers: Vec::new(),
            provider_binary_overrides: HashMap::new(),
            extra: BTreeMap::new(),
        }
    }
}

impl DaemonSettings {
    pub fn default_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(crate::identity::DATA_DIR_NAME)
            .join("settings.json")
    }

    /// Where the desktop files the interface language for the daemon.
    ///
    /// Not `language`: [`Self::discard_legacy_app_keys`] strips that one,
    /// because older builds dumped every app setting in here and the
    /// boundary was drawn deliberately. This is a different claim — the
    /// daemon renders user-facing text of its own (every driver's `tr!`
    /// call) and cannot do it in the user's language without being told.
    pub const LOCALE_KEY: &'static str = "sub2apiLocale";

    /// The interface language the desktop last pushed, if any.
    pub fn locale(&self) -> Option<&str> {
        self.extra.get(Self::LOCALE_KEY).and_then(Value::as_str)
    }

    pub fn set_locale(&mut self, locale: &str) {
        self.extra
            .insert(Self::LOCALE_KEY.to_owned(), Value::String(locale.to_owned()));
    }

    pub fn discard_legacy_app_keys(&mut self) {
        for key in ["analytics_enabled", "favorite_models", "theme", "language"] {
            self.extra.remove(key);
        }
    }
}
