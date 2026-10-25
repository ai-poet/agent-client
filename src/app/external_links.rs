//! Transcript links that leave the app.
//!
//! Fork addition. A link that is neither a project file nor a local path used
//! to go straight to `cx.open_url`, so whatever scheme an agent wrote reached
//! the operating system's URL handlers. It now goes through
//! [`crate::link_policy`], and a refused link says why in a toast.

use super::*;

impl Waku {
    pub(super) fn open_external_link(&mut self, target: &str, cx: &mut Context<Self>) {
        if let Err(reason) = crate::link_policy::open_checked(target, cx) {
            self.show_toast(tr!("errors.link_blocked", reason = reason.message()));
            cx.notify();
        }
    }
}
