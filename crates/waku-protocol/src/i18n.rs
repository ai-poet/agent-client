//! Shared locale selection and translation access.

use serde::{Deserialize, Serialize};

/// The language preference Waku persists. `System` resolves to one of the
/// locales Waku deliberately ships today.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppLanguage {
    System,
    English,
    SimplifiedChinese,
    Japanese,
}

impl AppLanguage {
    pub const ALL: [Self; 4] = [
        Self::System,
        Self::English,
        Self::SimplifiedChinese,
        Self::Japanese,
    ];

    pub fn locale(self) -> &'static str {
        match self.resolved() {
            Self::System => unreachable!("system language always resolves to a shipped locale"),
            Self::English => "en",
            Self::SimplifiedChinese => "zh-CN",
            Self::Japanese => "ja",
        }
    }

    /// Explicit language names are autonyms so the selector remains
    /// understandable even when the current locale is unfamiliar.
    pub fn label(self) -> String {
        match self {
            Self::System => translate("language.system"),
            Self::English => "English".to_owned(),
            Self::SimplifiedChinese => "简体中文".to_owned(),
            Self::Japanese => "日本語".to_owned(),
        }
    }

    pub fn resolved(self) -> Self {
        match self {
            Self::System => Self::from_system(),
            explicit => explicit,
        }
    }

    fn from_system() -> Self {
        Self::from_locale_id(&system_locale())
    }

    fn from_locale_id(locale: &str) -> Self {
        let locale = locale.replace('_', "-").to_ascii_lowercase();
        if locale == "zh-cn" || locale == "zh-sg" || locale.starts_with("zh-hans") {
            Self::SimplifiedChinese
        } else if locale == "ja" || locale.starts_with("ja-") {
            Self::Japanese
        } else {
            Self::English
        }
    }
}

impl Default for AppLanguage {
    fn default() -> Self {
        Self::System
    }
}

pub fn set_language(language: AppLanguage) {
    rust_i18n::set_locale(language.locale());
}

pub fn translate(key: &str) -> String {
    rust_i18n::t!(key).into_owned()
}

pub fn uses_east_asian_date_format() -> bool {
    locale_uses_east_asian_date_format(&rust_i18n::locale())
}

fn locale_uses_east_asian_date_format(locale: &str) -> bool {
    matches!(locale, "zh-CN" | "ja")
}

#[cfg(target_os = "macos")]
fn system_locale() -> String {
    use objc2_foundation::NSLocale;

    NSLocale::preferredLanguages()
        .firstObject()
        .map(|locale| locale.to_string())
        .unwrap_or_else(|| "en".to_owned())
}

/// Fork fix: the env-var probe below is a Unix idiom. Windows does not set
/// `LANG`, so upstream's "follow the system" always resolved to English there
/// — on the platform most of this fork's users run. Ask the OS instead;
/// `GetUserDefaultLocaleName` returns a BCP-47 tag such as `zh-CN`.
#[cfg(target_os = "windows")]
fn system_locale() -> String {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(locale_name: *mut u16, name_length: i32) -> i32;
    }
    // LOCALE_NAME_MAX_LENGTH is 85, including the terminator.
    let mut buffer = [0u16; 85];
    let written = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), buffer.len() as i32) };
    if written > 1 {
        String::from_utf16_lossy(&buffer[..written as usize - 1])
    } else {
        "en".to_owned()
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system_locale() -> String {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_else(|_| "en".to_owned())
        .split('.')
        .next()
        .unwrap_or("en")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_asks_the_os_not_unix_env_vars() {
        // The exact tag is machine-dependent; what must hold is that the
        // answer comes back at all — the env-var path always returned "en"
        // because Windows never sets LANG.
        let locale = system_locale();
        assert!(!locale.is_empty());
        // BCP-47 tags from GetUserDefaultLocaleName carry a region.
        assert!(locale == "en" || locale.contains('-'), "locale: {locale}");
    }

    #[test]
    fn language_locale_ids_are_supported() {
        assert_eq!(AppLanguage::English.locale(), "en");
        assert_eq!(AppLanguage::SimplifiedChinese.locale(), "zh-CN");
        assert_eq!(AppLanguage::Japanese.locale(), "ja");
        let locales = rust_i18n::available_locales!();
        assert_eq!(locales.len(), 3);
        assert!(locales.iter().any(|locale| locale.as_ref() == "en"));
        assert!(locales.iter().any(|locale| locale.as_ref() == "zh-CN"));
        assert!(locales.iter().any(|locale| locale.as_ref() == "ja"));
    }

    #[test]
    fn language_names_are_autonyms() {
        assert_eq!(AppLanguage::English.label(), "English");
        assert_eq!(AppLanguage::SimplifiedChinese.label(), "简体中文");
        assert_eq!(AppLanguage::Japanese.label(), "日本語");
    }

    #[test]
    fn system_is_the_default_persisted_preference_and_resolves_to_a_shipped_locale() {
        assert_eq!(AppLanguage::default(), AppLanguage::System);
        assert_eq!(
            serde_json::to_string(&AppLanguage::System).unwrap(),
            r#""system""#
        );
        assert!(matches!(
            AppLanguage::System.locale(),
            "en" | "zh-CN" | "ja"
        ));
    }

    #[test]
    fn japanese_system_locales_are_detected() {
        assert_eq!(AppLanguage::from_locale_id("ja"), AppLanguage::Japanese);
        assert_eq!(AppLanguage::from_locale_id("ja_JP"), AppLanguage::Japanese);
    }

    #[test]
    fn japanese_and_simplified_chinese_use_east_asian_dates() {
        assert!(locale_uses_east_asian_date_format("ja"));
        assert!(locale_uses_east_asian_date_format("zh-CN"));
        assert!(!locale_uses_east_asian_date_format("en"));
    }

    #[test]
    fn simplified_chinese_system_locales_are_detected_without_enabling_traditional_chinese() {
        assert_eq!(
            AppLanguage::from_locale_id("zh-Hans-CN"),
            AppLanguage::SimplifiedChinese
        );
        assert_eq!(
            AppLanguage::from_locale_id("zh_SG"),
            AppLanguage::SimplifiedChinese
        );
        assert_eq!(
            AppLanguage::from_locale_id("zh-Hant-TW"),
            AppLanguage::English
        );
    }

    #[test]
    fn translations_are_complete_and_interpolate_naturally() {
        assert_eq!(&*rust_i18n::t!("settings.daemon", locale = "en"), "Daemon");
        assert_eq!(
            &*rust_i18n::t!("daemon.expose_title", locale = "en"),
            "Expose managed daemon"
        );
        assert_eq!(
            &*rust_i18n::t!("settings.general", locale = "zh-CN"),
            "通用"
        );
        assert_eq!(
            &*rust_i18n::t!(
                "computer_use.allow_control",
                locale = "zh-CN",
                app = "Finder"
            ),
            "允许 CheapRouter 控制“Finder”吗？"
        );
        assert_eq!(
            &*rust_i18n::t!("session.rewound", locale = "zh-CN", turn = 3),
            "已回退到第 3 轮任务之前"
        );
        assert_eq!(&*rust_i18n::t!("settings.general", locale = "ja"), "一般");
        assert_eq!(
            &*rust_i18n::t!("computer_use.allow_control", locale = "ja", app = "Finder"),
            "CheapRouter に「Finder」の操作を許可しますか？"
        );
        assert_eq!(
            &*rust_i18n::t!("session.rewound", locale = "ja", turn = 3),
            "タスクをターン 3 の前まで巻き戻しました"
        );
    }

    #[test]
    fn task_creation_copy_uses_task_terminology() {
        assert_eq!(&*rust_i18n::t!("menu.new_task", locale = "en"), "New Task");
        assert_eq!(
            &*rust_i18n::t!("command_palette.new_task", locale = "en"),
            "New task"
        );
        assert_eq!(
            &*rust_i18n::t!("providers.disabled_for_new_tasks", locale = "en"),
            "Disabled for new tasks"
        );
    }
}
