//! Single source of product branding.
//!
//! Every brand-specific string in the fork resolves here, so rebranding is a
//! change to this file (or to the build environment) and nothing else. Each
//! constant reads a build-time environment variable first and falls back to the
//! compiled-in default, letting CI produce a differently-branded build without
//! editing tracked source.

/// Product name shown in window titles, the about box, and the user agent.
pub const DISPLAY_NAME: &str = match option_env!("SUB2API_BRAND_NAME") {
    Some(name) => name,
    None => "CheapRouter",
};

/// Reverse-DNS bundle identifier (macOS bundle id, Windows registry key).
pub const BUNDLE_ID: &str = match option_env!("SUB2API_BRAND_BUNDLE_ID") {
    Some(id) => id,
    None => "org.cheaprouter.desktop",
};

/// Marketing site, linked from the about box and error surfaces.
pub const WEBSITE_URL: &str = match option_env!("SUB2API_BRAND_WEBSITE") {
    Some(url) => url,
    None => "https://cheaprouter.org",
};

/// Managed cloud service base URL. All `/api/v1` calls are built from this.
pub const MANAGED_SERVICE_URL: &str = match option_env!("SUB2API_MANAGED_SERVICE_URL") {
    Some(url) => url,
    None => "https://cheaprouter.org",
};

/// Host serving the Sparkle appcast and release artifacts.
pub const RELEASES_BASE_URL: &str = match option_env!("SUB2API_RELEASES_BASE_URL") {
    Some(url) => url,
    None => "https://releases.cheaprouter.org",
};

/// Application data directory name under the user's home directory.
///
/// Kept as upstream's `.waku` by default: changing it would mean touching
/// several upstream files (`persistence.rs`, `projectless.rs`, `daemon.rs`) for
/// no user-visible gain, and this fork ships no data migration. Override only
/// if the fork must coexist with an upstream Waku install on one machine.
pub const DATA_DIR_NAME: &str = match option_env!("SUB2API_DATA_DIR_NAME") {
    Some(name) => name,
    None => ".waku",
};

/// Whether upstream's analytics client should be constructed at all.
///
/// Defaults to off: the fork has no telemetry endpoint of its own, and
/// reporting to upstream's would both pollute their data and leak our users'.
pub const ANALYTICS_ENABLED: bool = option_env!("SUB2API_ANALYTICS_ENABLED").is_some();

/// Absolute path to the application data directory.
pub fn data_dir() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|home| home.join(DATA_DIR_NAME))
}

/// Sparkle appcast URL for the running platform and architecture.
///
/// macOS reads its feed from `Info.plist`; this covers the Windows updater,
/// which resolves the URL in Rust.
pub fn appcast_url() -> String {
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    if cfg!(target_os = "windows") {
        format!("{RELEASES_BASE_URL}/appcast-windows-{arch}.xml")
    } else {
        format!("{RELEASES_BASE_URL}/appcast.xml")
    }
}

/// User agent sent with every managed-service request.
pub fn user_agent(version: &str) -> String {
    format!(
        "{DISPLAY_NAME}/{version} ({}; {})",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_service_url_has_no_trailing_slash() {
        // `api_url` joins with "/api/v1/...", so a trailing slash would produce
        // a double slash and a 404 on some reverse proxies.
        assert!(!MANAGED_SERVICE_URL.ends_with('/'));
        assert!(!RELEASES_BASE_URL.ends_with('/'));
        assert!(!WEBSITE_URL.ends_with('/'));
    }

    #[test]
    fn appcast_url_is_absolute() {
        let url = appcast_url();
        assert!(url.starts_with("https://"), "appcast url: {url}");
        assert!(url.ends_with(".xml"), "appcast url: {url}");
    }

    #[test]
    fn user_agent_carries_product_and_version() {
        let agent = user_agent("1.2.3");
        assert!(agent.starts_with(DISPLAY_NAME));
        assert!(agent.contains("1.2.3"));
    }

    #[test]
    fn bundle_id_is_reverse_dns() {
        assert!(BUNDLE_ID.split('.').count() >= 3, "bundle id: {BUNDLE_ID}");
    }
}
