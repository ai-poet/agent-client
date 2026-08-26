//! Shared application identity used by the daemon and desktop client.
//!
//! Fork change: the display name carries the brand. It reads the same
//! build-time variable as `sub2api::brand::DISPLAY_NAME` (this crate cannot
//! depend on that one), with the same compiled-in fallback — keep the two
//! defaults identical.
//!
//! `APP_ID` and `DATA_DIRECTORY_NAME` stay as upstream's on purpose: the id
//! must match the window-manager class the Linux desktop entry declares and
//! the platform identity already registered on users' machines, and renaming
//! the data directory would orphan existing state for zero user-visible gain.

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

#[cfg(debug_assertions)]
pub const DATA_DIRECTORY_NAME: &str = "Waku Debug";
#[cfg(not(debug_assertions))]
pub const DATA_DIRECTORY_NAME: &str = "Waku";
