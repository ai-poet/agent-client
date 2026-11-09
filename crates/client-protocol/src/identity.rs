//! Shared application identity used by the daemon and desktop client.
//!
//! Fork change: the display name and both data-directory names carry the
//! brand. `APP_NAME` and `DATA_DIR_NAME` read the same build-time variables
//! as `sub2api::brand` (this crate cannot depend on that one), with the same
//! compiled-in fallbacks — keep the defaults identical in both places.
//! Legacy upstream-named directories (`~/.waku`, `Waku`, `Waku Debug`) are
//! renamed in place at startup by `sub2api::migrate::migrate_legacy_storage`.
//!
//! `APP_ID` stays as upstream's on purpose: the id must match the
//! window-manager class the Linux desktop entry declares and the platform
//! identity already registered on users' machines.

#[cfg(debug_assertions)]
pub const APP_NAME: &str = match option_env!("SUB2API_BRAND_NAME") {
    Some(name) => name,
    None => "CheapRouter Debug",
};
#[cfg(not(debug_assertions))]
pub const APP_NAME: &str = match option_env!("SUB2API_BRAND_NAME") {
    Some(name) => name,
    None => "CheapRouter",
};

#[cfg(debug_assertions)]
pub const APP_ID: &str = "sh.waku.dev";
#[cfg(not(debug_assertions))]
pub const APP_ID: &str = "sh.waku";

/// Home-directory dot-folder holding settings, sessions, projectless
/// workspaces and worktrees (`~/.cheaprouter`).
pub const DATA_DIR_NAME: &str = match option_env!("SUB2API_DATA_DIR_NAME") {
    Some(name) => name,
    None => ".cheaprouter",
};

#[cfg(debug_assertions)]
pub const DATA_DIRECTORY_NAME: &str = "CheapRouter Debug";
#[cfg(not(debug_assertions))]
pub const DATA_DIRECTORY_NAME: &str = "CheapRouter";
